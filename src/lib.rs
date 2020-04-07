use std::sync::Arc;

use hyper::{Body, Client, Request, Response};
use hyper::client::HttpConnector;

pub mod proxy;
use proxy::{ProxyConfig, ScheduleConfigReload};

/// See documentation for struct `Proxy` fields.
pub async fn on_request(
    req: Request<Body>,
    client: Arc<Client<HttpConnector>>,
    proxy_config: Arc<ProxyConfig>,
    schedule_config_reload: ScheduleConfigReload,
) -> Result<Response<Body>, hyper::Error> {
    println!("proxy config: {:#?}", proxy_config);
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

/// Update request's URI to point to another server according to predefined routes.
fn handle_routes(mut req: Request<Body>, proxy_config: &ProxyConfig) -> Result<Request<Body>, Response<Body>> {
    *req.uri_mut() = "http://localhost:8000/".parse().unwrap();
    Ok(req)
}



