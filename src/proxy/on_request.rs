use std::collections::hash_map::DefaultHasher;
use std::convert::TryFrom;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use hyper::body::Bytes;
use hyper::client::HttpConnector;
use hyper::{header, Body, Client, Request, Response};
use hyper_timeout::TimeoutConnector;
use hyper_tls::HttpsConnector;

use http::{HeaderMap, Method, StatusCode, Uri};

use cache_control::CacheControl;
use serde::{Deserialize, Serialize};

use crate::helpers::now_timestamp;
use crate::hyper_helpers::{
    body_to_bytes, bytes_to_body, clone_request, fork_response, map_request_body,
};
use crate::proxy::validations;
use crate::proxy::{Db, ProxyConfig, ScheduleConfigReload};

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
    timestamp: i64,
    // Cached response is valid for `validity` seconds.
    validity: u32,
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
    timestamp: i64,
    // Cached response is valid for `validity` seconds.
    validity: u32,
}

// ------ on_request ------

type OnRequestClient = Arc<Client<TimeoutConnector<HttpsConnector<HttpConnector>>>>;

/// See documentation for struct `Proxy` fields.
///
/// # Errors
///
/// Returns error when HTTP stream handling fails.
pub async fn on_request(
    req: Request<Body>,
    client: OnRequestClient,
    proxy_config: Arc<ProxyConfig>,
    schedule_config_reload: ScheduleConfigReload,
    db: Db,
) -> Result<Response<Body>, hyper::Error> {
    if proxy_config.verbose {
        println!("original req: {:#?}", req);
    }

    let req = map_request_body(req, body_to_bytes).await?;

    let req_or_response =
        apply_request_middlewares(req, &proxy_config, &schedule_config_reload, &db);

    if proxy_config.verbose {
        println!("mapped req or response: {:#?}", req_or_response);
    }

    match req_or_response {
        // A middleware failed or it didn't want to send the given request -
        // just return prepared `Response`.
        Err(response) => Ok(response),
        // Send the modified request.
        Ok(req) => send_request_and_handle_response(req, &client, &proxy_config, &db).await,
    }
}

/// Send the request to origin and handle request fails and origin response.
async fn send_request_and_handle_response(
    req: Request<Bytes>,
    client: &OnRequestClient,
    proxy_config: &ProxyConfig,
    db: &Db,
) -> Result<Response<Body>, hyper::Error> {
    let response_db_key = CacheKey {
        method: req.method(),
        uri: req.uri(),
        body: req.body(),
    }
    .to_db_key();

    // We need to clone the request so we can use it later, when the request or response fails,
    // so we can try to get at least cached response.
    let req_clone = clone_request(&req);

    // We need to convert `Request<Bytes>` to `Request<Body>` to send it.
    let req = map_request_body(req, bytes_to_body).await?;

    // Send request.
    match client.request(req).await {
        Ok(response) => {
            if !validations::validate_response(&response) {
                return Ok(handle_origin_fail(&req_clone, proxy_config, db));
            }
            if !proxy_config.cache_enabled {
                if proxy_config.verbose {
                    println!("original response: {:#?}", response);
                }
                return Ok(response);
            }
            cache_response(response, response_db_key, proxy_config, db).await
        }
        // Request failed - return the response without caching.
        Err(error) => {
            eprintln!("Request error: {:#?}", error);
            Ok(handle_origin_fail(&req_clone, proxy_config, db))
        }
    }
}

/// Request to origin failed (e.g. timeout) or the response is invalid.
fn handle_origin_fail(req: &Request<Bytes>, proxy_config: &ProxyConfig, db: &Db) -> Response<Body> {
    let cache_key = CacheKey {
        method: req.method(),
        uri: req.uri(),
        body: req.body(),
    };

    match db.get(cache_key.to_db_key()) {
        // The cached response has been found.
        Ok(Some(cached_response)) => {
            match bincode::deserialize::<CacheValueForDeserialization>(cached_response.as_ref()) {
                // Return the cached response.
                Ok(cached_response) => {
                    if now_timestamp() - cached_response.timestamp
                        > i64::from(proxy_config.cache_stale_threshold_on_fail)
                    {
                        let mut response = Response::new(Body::from(
                            "No valid response. Cached response too old.",
                        ));
                        *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                        return response;
                    }

                    if proxy_config.verbose {
                        println!("response has been successfully loaded from the cache");
                    }

                    let mut response = Response::new(Body::from(cached_response.body));
                    *response.status_mut() = cached_response.status;
                    *response.headers_mut() = cached_response.headers;
                    response
                }
                // Deserialization failed.
                Err(error) => {
                    eprintln!("cannot deserialize a response`: {}", error);
                    let mut response =
                        Response::new(Body::from("Cannot deserialize a cached response."));
                    *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                    response
                }
            }
        }

        // The cached response hasn't been found.
        Ok(None) => {
            // We weren't able to get a fresh response and there isn't a cached one.
            let mut response = Response::new(Body::from("No valid response."));
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            response
        }

        // DB reading failed.
        Err(error) => {
            eprintln!("cannot read from DB`: {}", error);
            let mut response = Response::new(Body::from("Cannot read from the cache."));
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            response
        }
    }
}

/// Cache response.
///
/// _Note:_: It only logs cache errors because it's not a reason to not deliver response to the user.
async fn cache_response(
    response: Response<Body>,
    response_db_key: [u8; 8],
    proxy_config: &ProxyConfig,
    db: &Db,
) -> Result<Response<Body>, hyper::Error> {
    let (response, response_with_byte_body) = fork_response(response).await?;

    let serialization_result = bincode::serialize(&CacheValueForSerialization {
        status: response_with_byte_body.status(),
        headers: response_with_byte_body.headers(),
        body: response_with_byte_body.body(),
        timestamp: now_timestamp(),
        validity: validity_from_response(&response, proxy_config),
    });
    match serialization_result {
        Err(error) => {
            eprintln!("cannot serialize response: {}", error);
        }
        Ok(cache_value) => {
            // Try to cache the response.
            if let Err(error) = db.insert(response_db_key, cache_value) {
                eprintln!("cannot cache response with the key: {}", error);
            } else if proxy_config.verbose {
                println!("response has been successfully cached");
            }
        }
    }
    if proxy_config.verbose {
        println!("original and just cached response: {:#?}", response);
    }
    Ok(response)
}

/// Get `validity` from cache headers or use the default value from `ProxyConfig`.
fn validity_from_response(response: &Response<Body>, proxy_config: &ProxyConfig) -> u32 {
    // Try to get the value from `Cache-Control: max-age=<seconds>`,
    // where `seconds` is `u32`.
    response
        .headers()
        .get(header::CACHE_CONTROL)
        .and_then(|header_value| header_value.to_str().ok())
        .and_then(CacheControl::from_value)
        .and_then(|cache_control| cache_control.max_age)
        .and_then(|duration| u32::try_from(duration.num_seconds()).ok())
        .unwrap_or(proxy_config.default_cache_validity)
}

/// Aka "middleware pipeline".
fn apply_request_middlewares(
    mut req: Request<Bytes>,
    proxy_config: &ProxyConfig,
    schedule_config_reload: &ScheduleConfigReload,
    db: &Db,
) -> Result<Request<Bytes>, Response<Body>> {
    req = handle_config_reload(req, proxy_config, schedule_config_reload)?;
    req = handle_clear_cache(req, proxy_config, db)?;
    req = handle_status(req, proxy_config)?;
    req = handle_routes(req, proxy_config)?;
    if proxy_config.cache_enabled {
        req = handle_cache(req, db, proxy_config.verbose)?;
    }
    Ok(req)
}

/// Schedule proxy config reload and return simple 200 response when the predefined URL path is matched.
fn handle_config_reload(
    req: Request<Bytes>,
    proxy_config: &ProxyConfig,
    schedule_config_reload: &ScheduleConfigReload,
) -> Result<Request<Bytes>, Response<Body>> {
    if req.uri().path() == proxy_config.reload_config_url_path {
        schedule_config_reload();
        return Err(Response::new(Body::from("Proxy config reload scheduled.")));
    }
    Ok(req)
}

/// Clear cache and return simple 200 response when the predefined URL path is matched.
fn handle_clear_cache(
    req: Request<Bytes>,
    proxy_config: &ProxyConfig,
    db: &Db,
) -> Result<Request<Bytes>, Response<Body>> {
    if req.uri().path() == proxy_config.clear_cache_url_path {
        if let Err(error) = db.clear() {
            eprintln!("cache clearing failed: {}", error);
            return Err(Response::new(Body::from("Cache clearing failed.")));
        }
        return Err(Response::new(Body::from("Cache cleared.")));
    }
    Ok(req)
}

/// Return response with text "Proxy is ready." when the predefined URL path is matched.
fn handle_status(
    req: Request<Bytes>,
    proxy_config: &ProxyConfig,
) -> Result<Request<Bytes>, Response<Body>> {
    if req.uri().path() == proxy_config.status_url_path {
        return Err(Response::new(Body::from("Proxy is ready.")));
    }
    Ok(req)
}

/// Update request's URI to point to another address according to predefined routes.
///
/// # Errors
///
/// - Returns 200 and the content of `landing.html` when the incoming request does not match any routes.
/// - Returns `BAD_REQUEST` when request validation fails.
/// - Returns `INTERNAL_SERVER_ERROR` response if the new address is invalid.
fn handle_routes(
    mut req: Request<Bytes>,
    proxy_config: &ProxyConfig,
) -> Result<Request<Bytes>, Response<Body>> {
    let uri = req.uri();
    // Try to get the host directly from `req.uri`, then from `host` header and then represent it as relative url.
    let host = uri
        .host()
        .or_else(|| {
            req.headers()
                .get("host")
                .and_then(|value| value.to_str().ok())
        })
        .unwrap_or_default();

    // http://example.com/abc/efg?x=1&y=2 -> example.com/abc/efg?x=1&y=2
    let from = format!("{}{}{}", host, uri.path(), uri.query().unwrap_or_default());

    // Get the first matching route or return 404 / a landing file.
    let route = proxy_config
        .routes
        .iter()
        .find(|route| from.starts_with(&route.from));
    let route = match route {
        Some(route) => route,
        None => {
            if uri.path() == "/" {
                // Return `landing.html`.
                let response =
                    Response::new(Body::from(include_bytes!("../../landing.html").as_ref()));
                return Err(response);
            } else {
                // Return 404
                let mut response = Response::new(Body::from(
                    "404. The requested URL was not found on this server.",
                ));
                *response.status_mut() = StatusCode::NOT_FOUND;
                return Err(response);
            }
        }
    };

    // @TODO: Replace `trim_start_matches` with `strip_prefix` once stable.
    // example.com/abc/efg?x=1&y=2 -> /abc/efg?x=1&y=2  (if matching route's `from` is "example.com")
    let routed_path_and_query = from.trim_start_matches(&route.from);

    // Request validation.
    if route.validate != Some(false) && !validations::validate_request(&req, routed_path_and_query)
    {
        let mut response = Response::new(Body::from("Invalid request."));
        *response.status_mut() = StatusCode::BAD_REQUEST;
        return Err(response);
    }

    // @TODO: Replace `trim_start_matches` with `strip_prefix` once stable.
    // /abc/efg?x=1&y=2 -> http://localhost:8000/abc/efgx=1&y=2 (if matching route's `to` is "http://localhost:8000")
    *req.uri_mut() = match format!(
        "{}{}",
        route.to,
        routed_path_and_query.trim_start_matches('/')
    )
    .parse()
    {
        Ok(uri) => uri,
        Err(error) => {
            eprintln!("Invalid URI in `handle_routes`: {}", error);
            let mut response = Response::new(Body::from("Cannot route to invalid URI."));
            *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            return Err(response);
        }
    };

    // Replace `host` header with the new one from `Request`'s `uri`.
    if let Some(host) = req.uri().host().and_then(|host| host.parse().ok()) {
        req.headers_mut().insert("host", host);
    } else {
        eprintln!("Missing host in the request uri: {}", req.uri());
        let mut response = Response::new(Body::from("Cannot route to URI without host."));
        *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        return Err(response);
    }

    Ok(req)
}

/// Return cached response if possible.
///
/// # Errors
/// - Returns cached response.
/// - Returns `INTERNAL_SERVER_ERROR` response when DB reading fails.
/// - Returns `INTERNAL_SERVER_ERROR` response when deserialization of a cached response fails.
fn handle_cache(
    req: Request<Bytes>,
    db: &Db,
    verbose: bool,
) -> Result<Request<Bytes>, Response<Body>> {
    let cache_key = CacheKey {
        method: req.method(),
        uri: req.uri(),
        body: req.body(),
    };

    match db.get(cache_key.to_db_key()) {
        // The cached response has been found.
        Ok(Some(cached_response)) => {
            Err(
                match bincode::deserialize::<CacheValueForDeserialization>(cached_response.as_ref())
                {
                    // Return the cached response.
                    Ok(cached_response) => {
                        // Is cached response still valid?
                        if now_timestamp()
                            > cached_response.timestamp + i64::from(cached_response.validity)
                        {
                            return Ok(req);
                        }

                        if verbose {
                            println!("response has been successfully loaded from the cache");
                        }

                        let mut response = Response::new(Body::from(cached_response.body));
                        *response.status_mut() = cached_response.status;
                        *response.headers_mut() = cached_response.headers;
                        response
                    }
                    // Deserialization failed.
                    Err(error) => {
                        eprintln!("Cannot deserialize a response`: {}", error);
                        let mut response =
                            Response::new(Body::from("Cannot deserialize a cached response."));
                        *response.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                        response
                    }
                },
            )
        }

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

// ------ ------- TESTS ------ ------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProxyRoute;
    use std::net::{IpAddr, Ipv4Addr};
    use std::path::PathBuf;

    // ------ handle_status ------

    #[tokio::test]
    async fn status() {
        let request = Request::builder()
            .uri("https://example.com/status")
            .body(Bytes::new())
            .unwrap();
        let config = default_proxy_config();

        let response = handle_status(request, &config).unwrap_err();
        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_bytes(response.into_body()).await.unwrap();
        assert_eq!(body, "Proxy is ready.");
    }

    // ------ handle_routes ------

    #[tokio::test]
    async fn handle_routes_unknown_root() {
        let request = Request::builder()
            .uri("https://example.com")
            .body(Bytes::new())
            .unwrap();
        let config = default_proxy_config();

        let response = handle_routes(request, &config).unwrap_err();
        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_bytes(response.into_body()).await.unwrap();
        assert_eq!(body, include_str!("../../landing.html"));
    }

    #[tokio::test]
    async fn handle_routes_unknown_root_slash() {
        let request = Request::builder()
            .uri("https://example.com/")
            .body(Bytes::new())
            .unwrap();
        let config = default_proxy_config();

        let response = handle_routes(request, &config).unwrap_err();
        assert_eq!(response.status(), StatusCode::OK);

        let body = body_to_bytes(response.into_body()).await.unwrap();
        assert_eq!(body, include_str!("../../landing.html"));
    }

    #[tokio::test]
    async fn handle_routes_unknown() {
        let request = Request::builder()
            .uri("https://example.com/unknown")
            .body(Bytes::new())
            .unwrap();
        let config = default_proxy_config();

        let response = handle_routes(request, &config).unwrap_err();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = body_to_bytes(response.into_body()).await.unwrap();
        assert_eq!(body, "404. The requested URL was not found on this server.");
    }

    #[tokio::test]
    async fn handle_routes_manifest() {
        let request = Request::builder()
            .uri("https://example.com/manifest.json")
            .body(Bytes::new())
            .unwrap();
        let mut config = default_proxy_config();
        config.routes.push(ProxyRoute {
            from: "example.com".to_owned(),
            to: "http://localhost:8080".parse().unwrap(),
            validate: None,
        });

        let request = handle_routes(request, &config).unwrap();
        assert_eq!(request.uri(), "http://localhost:8080/manifest.json");
    }

    #[tokio::test]
    async fn handle_routes_top() {
        let request = Request::builder()
            .uri("https://example.com/catalog/movie/top.json")
            .body(Bytes::new())
            .unwrap();
        let mut config = default_proxy_config();
        config.routes.push(ProxyRoute {
            from: "example.com".to_owned(),
            to: "http://localhost:8080".parse().unwrap(),
            validate: None,
        });

        let request = handle_routes(request, &config).unwrap();
        assert_eq!(
            request.uri(),
            "http://localhost:8080/catalog/movie/top.json"
        );
    }

    #[tokio::test]
    async fn handle_routes_invalid_validate() {
        let request = Request::builder()
            .uri("https://example.com/invalid")
            .body(Bytes::new())
            .unwrap();
        let mut config = default_proxy_config();
        config.routes.push(ProxyRoute {
            from: "example.com".to_owned(),
            to: "http://localhost:8080".parse().unwrap(),
            validate: None,
        });

        let response = handle_routes(request, &config).unwrap_err();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = body_to_bytes(response.into_body()).await.unwrap();
        assert_eq!(body, "Invalid request.");
    }

    #[tokio::test]
    async fn handle_routes_invalid() {
        let request = Request::builder()
            .uri("https://example.com/invalid")
            .body(Bytes::new())
            .unwrap();
        let mut config = default_proxy_config();
        config.routes.push(ProxyRoute {
            from: "example.com".to_owned(),
            to: "http://localhost:8080".parse().unwrap(),
            validate: Some(false),
        });

        let request = handle_routes(request, &config).unwrap();
        assert_eq!(request.uri(), "http://localhost:8080/invalid");
    }

    fn default_proxy_config() -> ProxyConfig {
        ProxyConfig {
            reload_config_url_path: "/reload-proxy-config".to_owned(),
            clear_cache_url_path: "/clear-cache".to_owned(),
            status_url_path: "/status".to_owned(),
            db_directory: PathBuf::from("proxy_db"),
            ip: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            default_port: 5000,
            cache_enabled: false,
            default_cache_validity: 600,            // 10 * 60
            cache_stale_threshold_on_fail: 172_800, // 48 * 60 * 60
            timeout: 20,
            routes: Vec::new(),
            verbose: false,
        }
    }
}
