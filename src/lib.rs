use std::sync::Arc;

use hyper::{Body, Client, Request, Response};
use hyper::client::HttpConnector;
use http::StatusCode;

pub mod proxy;
use proxy::{ProxyConfig, ScheduleConfigReload};

/// See documentation for struct `Proxy` fields.
pub async fn on_request(
    req: Request<Body>,
    client: Arc<Client<HttpConnector>>,
    proxy_config: Arc<ProxyConfig>,
    schedule_config_reload: ScheduleConfigReload,
) -> Result<Response<Body>, hyper::Error> {
    // println!("proxy config: {:#?}", proxy_config);
    println!("original req: {:#?}", req);

    let req = try_map_request(req, &proxy_config, schedule_config_reload);
    println!("mapped req or response: {:#?}", req);

    match req {
        Ok(req) => client.request(req).await,
        Err(response) => Ok(response)
    }
}

/// Aka "middleware pipeline".
fn try_map_request(mut req: Request<Body>, proxy_config: &ProxyConfig, schedule_config_reload: ScheduleConfigReload) -> Result<Request<Body>, Response<Body>> {
    req = handle_config_reload(req, proxy_config, schedule_config_reload)?;
    req = handle_routes(req, proxy_config)?;
    Ok(req)
}

/// Schedule proxy config reload and return simple 200 response when the predefined URL path is matched.
fn handle_config_reload(req: Request<Body>, proxy_config: &ProxyConfig, schedule_config_reload: ScheduleConfigReload) -> Result<Request<Body>, Response<Body>> {
    if req.uri().path() == proxy_config.reload_config_url_path {
        schedule_config_reload();
        return Err(Response::new(Body::from("Proxy config reload scheduled.")))
    }
    Ok(req)
}

/// Update request's URI to point to another address according to predefined routes.
/// Returns BAD_REQUEST response if there is no matching route.
fn handle_routes(mut req: Request<Body>, proxy_config: &ProxyConfig) -> Result<Request<Body>, Response<Body>> {
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
    let new_uri = format!("{}{}", route.to, routed_path_and_query);

    *uri = new_uri.parse().expect("routed uri");
    Ok(req)
}



