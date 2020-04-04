use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use futures_util::future::try_join;

use hyper::service::{make_service_fn, service_fn};
use hyper::upgrade::Upgraded;
use hyper::{Body, Client, Method, Request, Response, Server};
use hyper::client::HttpConnector;

use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::task;

pub mod proxy;
use proxy::{Proxy, ProxyConfig};

/// `proxy_request` is invoked for each request.
/// It allows you to modify or validate original request.
/// You can return `Response` from the proxied endpoint, e.g.:
/// ```rust,no_run
/// client.request(req).await
/// ```
/// or you can return a custom `Response`, e.g.:
/// ```rust,no_run
/// Ok(Response::new(Body::from("Proxy config reload scheduled.")))
/// ```
///
/// # Parameters
///
/// - `req: Request<Body>` - The original request.
///
/// - `client: Arc<Client<HttpConnector>>` - The client set in `Proxy` instance.
///    `Client` type parameters can be changed to support, for instance, TLS.
///
/// - `proxy_config: Arc<ProxyConfig>` - A configuration loaded from `proxy_config.toml`.
///
/// - `schedule_config_reload: impl Fn()` - The configuration will be reloaded and passed
///    to new requests after `schedule_config_reload` call.
///
/// # Errors
/// Returns `hyper::Error` when request fails.
pub async fn proxy_request(
    req: Request<Body>,
    client: Arc<Client<HttpConnector>>,
    proxy_config: Arc<ProxyConfig>,
    schedule_config_reload: impl Fn(),
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
fn try_map_request(mut req: Request<Body>, proxy_config: &ProxyConfig, schedule_config_reload: impl Fn()) -> Result<Request<Body>, Response<Body>> {
    req = handle_config_reload(req, proxy_config, schedule_config_reload)?;
    req = handle_routes(req, proxy_config)?;
    Ok(req)
}

/// Schedule proxy config reload and return simple 200 response when the predefined URL path is matched.
fn handle_config_reload(req: Request<Body>, proxy_config: &ProxyConfig, schedule_config_reload: impl Fn()) -> Result<Request<Body>, Response<Body>> {
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



