use std::sync::Arc;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use hyper::{Body, Client, Request, Response};
use hyper::client::HttpConnector;
use hyper::body::Bytes;

use http::{StatusCode, Method, Uri, HeaderMap};
use http_serde;
use serde::{Deserialize, Serialize};
use bincode;

pub mod proxy;

use proxy::{ProxyConfig, ScheduleConfigReload, Db};

// ------ CacheKey ------

#[derive(Hash)]
/// Key for Sled DB.
struct CacheKey<'a> {
    method: &'a Method,
    uri: &'a Uri,
    body: &'a Bytes,
}

impl<'a> CacheKey<'a> {
    /// Convert to Sled DB compatible keys.
    ///
    /// _Notes:_
    ///   - Sled DB supports only `AsRef<u8>` as the keys and values.
    ///   - Big-endian is recommended by Sled DB docs.
    fn to_db_key(&self) -> [u8; 8] {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish().to_be_bytes()
    }
}

// ------ CacheValue ------

// @TODO: Can we merge it?

/// Value for Sled DB.
#[derive(Deserialize)]
struct CacheValueForDeserialization {
    #[serde(with = "http_serde::status_code")]
    status: StatusCode,
    #[serde(with = "http_serde::header_map")]
    headers: HeaderMap,
    #[serde(with = "serde_bytes")]
    body: Vec<u8>,
}

/// Value for Sled DB.
#[derive(Serialize)]
struct CacheValueForSerialization<'a> {
    #[serde(with = "http_serde::status_code")]
    status: StatusCode,
    #[serde(with = "http_serde::header_map")]
    headers: &'a HeaderMap,
    #[serde(with = "serde_bytes")]
    body: &'a [u8],
}


// ------ on_request ------

/// See documentation for struct `Proxy` fields.
pub async fn on_request(
    req: Request<Body>,
    client: Arc<Client<HttpConnector>>,
    proxy_config: Arc<ProxyConfig>,
    schedule_config_reload: ScheduleConfigReload,
    db: Db,
) -> Result<Response<Body>, hyper::Error> {
    // println!("proxy config: {:#?}", proxy_config);
    println!("original req: {:#?}", req);

    // @TODO: refactor (also below the similar code)
    let (parts, body) = req.into_parts();
    let body_bytes = hyper::body::to_bytes(body).await?;
    let req = Request::from_parts(parts, body_bytes);

    let req = try_map_request(req, &proxy_config, schedule_config_reload, &db);
    println!("mapped req or response: {:#?}", req);

    match req {
        // @TODO serialize and cache the response.
        Ok(req) => {
            let (parts, body) = req.into_parts();
            let req = Request::from_parts(parts, Body::from(body));



            match client.request(req).await {
                Ok(response) => {
                    let cache_value = bincode::serialize(&CacheValueForSerialization {
                        status: response.status(),
                        headers: response.headers(),
                        body: response.body().  // todo into parts to parts,
                    });
                    Ok(response)
                },
                error => error
            }


        },
        Err(response) => Ok(response)
    }
}

/// Aka "middleware pipeline".
fn try_map_request(
    mut req: Request<Bytes>,
    proxy_config: &ProxyConfig,
    schedule_config_reload: ScheduleConfigReload,
    db: &Db,
) -> Result<Request<Bytes>, Response<Body>> {
    req = handle_config_reload(req, proxy_config, schedule_config_reload)?;
    req = handle_routes(req, proxy_config)?;
    req = handle_cache(req, proxy_config, db)?;
    Ok(req)
}

/// Schedule proxy config reload and return simple 200 response when the predefined URL path is matched.
fn handle_config_reload(
    req: Request<Bytes>,
    proxy_config: &ProxyConfig,
    schedule_config_reload: ScheduleConfigReload
) -> Result<Request<Bytes>, Response<Body>> {
    if req.uri().path() == proxy_config.reload_config_url_path {
        schedule_config_reload();
        return Err(Response::new(Body::from("Proxy config reload scheduled.")))
    }
    Ok(req)
}

/// Update request's URI to point to another address according to predefined routes.
///
/// # Errors
///
/// - Returns BAD_REQUEST response if there is no matching route.
/// - Returns INTERNAL_SERVER_ERROR response if the new address is invalid.
fn handle_routes(mut req: Request<Bytes>, proxy_config: &ProxyConfig) -> Result<Request<Bytes>, Response<Body>> {
    let uri = req.uri_mut();
    // http://example.com/abc/efg?x=1&y=2 -> example.com/abc/efg?x=1&y=2
    let from = format!("{}{}{}", uri.host().unwrap_or_default(), uri.path(), uri.query().unwrap_or_default());

    // Get the first matching route or return BAD_REQUEST.
    let route = proxy_config.routes.iter().find(|route| {
        from.starts_with(&route.from)
    });
    let route = match route {
        Some(route) => route,
        None => {
            let mut response = Response::new(Body::from("No route matches."));
            *response.status_mut() = StatusCode::BAD_REQUEST;
            return Err(response)
        }
    };

    // @TODO: Replace `trim_start_matches` with `strip_prefix` once stable.
    // example.com/abc/efg?x=1&y=2 -> abc/efg?x=1&y=2  (if matching route's `from` is "example.com")
    let routed_path_and_query = from.trim_start_matches(&route.from).trim_start_matches("/");

    // abc/efg?x=1&y=2 -> http://localhost:8000/abc/efgx=1&y=2 (if matching route's `to` is "http://localhost:8000")
    *uri = match format!("{}{}", route.to, routed_path_and_query).parse() {
        Ok(uri) => uri,
        Err(error) => {
            eprintln!("Invalid URI in `handle_routes`: {}", error);
            let mut response = Response::new(Body::from("Cannot route to invalid URI."));
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            return Err(response)
        }
    };
    Ok(req)
}

/// Return cached response if possible.
///
/// # Errors
/// - Returns cached response.
/// - Returns INTERNAL_SERVER_ERROR response when DB reading fails.
/// - Returns INTERNAL_SERVER_ERROR response when deserialization of a cached response fails.
fn handle_cache(req: Request<Bytes>, _proxy_config: &ProxyConfig, db: &Db) -> Result<Request<Bytes>, Response<Body>> {
    let cache_key = CacheKey { method: req.method(), uri: req.uri(), body: req.body()};

    match db.get(cache_key.to_db_key()) {
        // The cached response has been found.
        Ok(Some(cached_response)) => {
            Err(match bincode::deserialize::<CacheValueForDeserialization>(cached_response.as_ref()) {
                // Return the cached response.
                Ok(cached_response) => {
                    let mut response = Response::new(Body::from(cached_response.body));
                    *response.status_mut() = cached_response.status;
                    *response.headers_mut() = cached_response.headers;
                    response
                },
                // Deserialization failed.
                Err(error) => {
                    eprintln!("Cannot deserialize a response`: {}", error);
                    let mut response = Response::new(Body::from("Cannot deserialize a cached response."));
                    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                    response
                }
            })
        },

        // The cached response hasn't been found => just return `req` without any changes.
        Ok(None) => Ok(req),

        // DB reading failed.
        Err(error) => {
            eprintln!("Cannot read from DB`: {}", error);
            let mut response = Response::new(Body::from("Cannot read from the cache."));
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            Err(response)
        }
    }
}



