use std::sync::Arc;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use hyper::{Body, Client, Request, Response};
use hyper::body::Bytes;
use hyper::client::HttpConnector;
use hyper_timeout::TimeoutConnector;

use hyper_tls::HttpsConnector;

use http::{StatusCode, Method, Uri, HeaderMap};
use http_serde;
use serde::{Deserialize, Serialize};
use bincode;

use crate::proxy::{ProxyConfig, ScheduleConfigReload, Db};
use crate::proxy::business;
use crate::hyper_helpers::{map_request_body, clone_request, body_to_bytes, bytes_to_body, fork_response};

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

type OnRequestClient = Arc<Client<TimeoutConnector<HttpsConnector<HttpConnector>>>>;

/// See documentation for struct `Proxy` fields.
pub async fn on_request(
    req: Request<Body>,
    client: OnRequestClient,
    proxy_config: Arc<ProxyConfig>,
    schedule_config_reload: ScheduleConfigReload,
    db: Db,
) -> Result<Response<Body>, hyper::Error> {
    // println!("proxy config: {:#?}", proxy_config);
    println!("original req: {:#?}", req);

    let req = map_request_body(req, body_to_bytes).await?;

    let req_or_response = apply_request_middlewares(
        req, &proxy_config, schedule_config_reload, &db
    );
    println!("mapped req or response: {:#?}", req_or_response);

    match req_or_response {
        // A middleware failed or it didn't want to send the given request -
        // just return prepared `Response`.
        Err(response) => Ok(response),
        // Send the modified request.
        Ok(req) => {
            send_request_and_handle_response(req, &client, &proxy_config, &db).await
        },
    }
}

/// Send the request to origin and handle request fails and origin response.
async fn send_request_and_handle_response(
    req: Request<Bytes>, 
    client: &OnRequestClient, 
    proxy_config: &ProxyConfig, 
    db: &Db
) -> Result<Response<Body>, hyper::Error> {
    let response_db_key = CacheKey { method: req.method(), uri: req.uri(), body: req.body()}
        .to_db_key();

    // We need to clone the request so we can use it later, when the request or response fails,
    // so we can try to get at least cached response.
    let req_clone = clone_request(&req);

    // We need to convert `Request<Bytes>` to `Request<Body>` to send it.
    let req = map_request_body(req, bytes_to_body).await?;

    // Send request.
    match client.request(req).await {
        Ok(response) => {
            if !business::validate_response(&response) {
                return Ok(handle_origin_fail(req_clone, db))
            }
            if !proxy_config.cache_enabled {
                println!("original response: {:#?}", response);
                return Ok(response)
            }
            cache_response(response, response_db_key, db).await
        },
        // Request failed - return the response without caching.
        Err(error) => {
            eprintln!("Request error: {:#?}", error);
            return Ok(handle_origin_fail(req_clone, db))
        }
    }
}

/// Request to origin failed (e.g. timeout) or the response is invalid.
fn handle_origin_fail(req: Request<Bytes>, db: &Db) -> Response<Body> {
    if let Err(response) = handle_cache(req, db) {
        // Return cached response or INTERNAL_SERVER_ERROR if something failed (DB or deserialization),
        return response
    }

    // We weren't able to get a fresh response and there isn't a cached one.
    let mut response = Response::new(Body::from("No valid response."));
    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    response
}

/// Cache response. 
/// It only logs cache errors because it's not a reason to not deliver response to the user.
async fn cache_response(
    response: Response<Body>, response_db_key: [u8; 8], db:&Db
) -> Result<Response<Body>, hyper::Error> {
    let (response, response_with_byte_body) = fork_response(response).await?;

    let serialization_result = bincode::serialize(
        &CacheValueForSerialization {
            status: response_with_byte_body.status(),
            headers: response_with_byte_body.headers(),
            body: response_with_byte_body.body(),
        }
    );
    match serialization_result {
        Err(error) => {
            eprintln!("cannot serialize response: {}", error);
        }
        Ok(cache_value) => {
            // Try to cache the response.
            if let Err(error) = db.insert(response_db_key, cache_value) {
                eprintln!("cannot cache response with the key: {}", error);
            } else {
                println!("response has been successfully cached");
            }
        }
    }
    println!("original and just cached response: {:#?}", response);
    Ok(response)
}

/// Aka "middleware pipeline".
fn apply_request_middlewares(
    mut req: Request<Bytes>,
    proxy_config: &ProxyConfig,
    schedule_config_reload: ScheduleConfigReload,
    db: &Db,
) -> Result<Request<Bytes>, Response<Body>> {
    req = handle_config_reload(req, proxy_config, schedule_config_reload)?;
    req = handle_clear_cache(req, proxy_config, db)?;
    req = handle_status(req, proxy_config)?;
    req = handle_routes(req, proxy_config)?;
    if proxy_config.cache_enabled {
        req = handle_cache(req, db)?;
    }
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

/// Clear cache and return simple 200 response when the predefined URL path is matched.
fn handle_clear_cache(
    req: Request<Bytes>,
    proxy_config: &ProxyConfig,
    db: &Db
) -> Result<Request<Bytes>, Response<Body>> {
    if req.uri().path() == proxy_config.clear_cache_url_path {
        if let Err(error) = db.clear() {
            eprintln!("cache clearing failed: {}", error);
            return Err(Response::new(Body::from("Cache clearing failed.")))
        }
        return Err(Response::new(Body::from("Cache cleared.")))
    }
    Ok(req)
}

/// Return response with text "Proxy is ready." when the predefined URL path is matched.
fn handle_status(
    req: Request<Bytes>,
    proxy_config: &ProxyConfig,
) -> Result<Request<Bytes>, Response<Body>> {
    if req.uri().path() == proxy_config.status_url_path {
        return Err(Response::new(Body::from("Proxy is ready.")))
    }
    Ok(req)
}

/// Update request's URI to point to another address according to predefined routes.
///
/// # Errors
///
/// - Returns 200 and the content of `landing.html` when the incoming request does not match any routes.
/// - Returns BAD_REQUEST when request validation fails.
/// - Returns INTERNAL_SERVER_ERROR response if the new address is invalid.
fn handle_routes(mut req: Request<Bytes>, proxy_config: &ProxyConfig) -> Result<Request<Bytes>, Response<Body>> {
    let uri = req.uri();
    // Try to get the host directly from `req.uri`, then from `host` header and then represent it as relative url.
    let host = uri.host()
        .or_else(|| req.headers().get("host").and_then(|value| value.to_str().ok()))
        .unwrap_or_default();

    // http://example.com/abc/efg?x=1&y=2 -> example.com/abc/efg?x=1&y=2
    let from = format!("{}{}{}", host, uri.path(), uri.query().unwrap_or_default());

    // Get the first matching route or return a landing file.
    let route = proxy_config.routes.iter().find(|route| {
        from.starts_with(&route.from)
    });
    let route = match route {
        Some(route) => route,
        None => {
            // Return `landing.html`.
            let response = Response::new(Body::from(include_bytes!("../../landing.html").as_ref()));
            return Err(response)
        }
    };

    // @TODO: Replace `trim_start_matches` with `strip_prefix` once stable.
    // example.com/abc/efg?x=1&y=2 -> /abc/efg?x=1&y=2  (if matching route's `from` is "example.com")
    let routed_path_and_query = from.trim_start_matches(&route.from);

    // Request validation.
    if route.validate != Some(false) && !business::validate_request(routed_path_and_query) {
        let mut response = Response::new(Body::from("Invalid request."));
        *response.status_mut() = StatusCode::BAD_REQUEST;
        return Err(response)
    }

    // @TODO: Replace `trim_start_matches` with `strip_prefix` once stable.
    // /abc/efg?x=1&y=2 -> http://localhost:8000/abc/efgx=1&y=2 (if matching route's `to` is "http://localhost:8000")
    *req.uri_mut() = match format!("{}{}", route.to, routed_path_and_query.trim_start_matches("/")).parse() {
        Ok(uri) => uri,
        Err(error) => {
            eprintln!("Invalid URI in `handle_routes`: {}", error);
            let mut response = Response::new(Body::from("Cannot route to invalid URI."));
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            return Err(response)
        }
    };

    // Replace `host` header with the new one from `Request`'s `uri`.
    match req.uri().host().and_then(|host| host.parse().ok()) {
        Some(host) => req.headers_mut().insert("host", host),
        None => {
            eprintln!("Missing host in the request uri: {}", req.uri());
            let mut response = Response::new(Body::from("Cannot route to URI without host."));
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
fn handle_cache(req: Request<Bytes>, db: &Db) -> Result<Request<Bytes>, Response<Body>> {
    let cache_key = CacheKey { method: req.method(), uri: req.uri(), body: req.body()};

    match db.get(cache_key.to_db_key()) {
        // The cached response has been found.
        Ok(Some(cached_response)) => {
            Err(match bincode::deserialize::<CacheValueForDeserialization>(cached_response.as_ref()) {
                // Return the cached response.
                Ok(cached_response) => {
                    println!("response has been successfully loaded from the cache");
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



