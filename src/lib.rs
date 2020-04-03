use std::convert::Infallible;
use std::net::SocketAddr;

use futures_util::future::try_join;

use hyper::service::{make_service_fn, service_fn};
use hyper::upgrade::Upgraded;
use hyper::{Body, Client, Method, Request, Response, Server};

use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::task;

pub mod proxy;
use proxy::{Proxy, ProxyConfig, HttpClient};

pub async fn proxy_request(
    mut req: Request<Body>,
    client: HttpClient,
    proxy_config: ProxyConfig,
    schedule_config_refresh: impl Fn(),
) -> Result<Response<Body>, hyper::Error> {
    schedule_config_refresh();

    println!("req: {:#?}", req);

    req = map_request(req);
    client.request(req).await
}

fn map_request(mut req: Request<Body>) -> Request<Body> {
    route(req.uri_mut());
    req
}

fn route(uri: &mut http::Uri) {
    *uri = "http://localhost:8000/".parse().unwrap();
}



