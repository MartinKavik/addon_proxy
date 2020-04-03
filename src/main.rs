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

mod proxy;

use proxy::ProxyConfig;

type HttpClient = Client<hyper::client::HttpConnector>;

macro_rules! shadow_clone {
    ($ ($to_clone:ident) ,*) => {
        $(
            #[allow(unused_mut)]
            let mut $to_clone = $to_clone.clone();
        )*
    };
}

#[tokio::main]
async fn main() {
    // @TODO init db?
    // https://github.com/TheNeikos/rustbreak
    // https://github.com/spacejam/sled

    let proxy_config = ProxyConfig::load().await;
    let addr = proxy_config.socket_address.clone();

    let (config_refresh_sender, mut config_refresh_receiver) = mpsc::unbounded_channel();
    let (config_sender, config_receiver) = watch::channel(proxy_config);

    task::spawn(async move {
        while let Some(_) = config_refresh_receiver.recv().await {
            config_sender.broadcast(ProxyConfig::load().await).expect("broadcast proxy config")
        }
    });

    let service = service_fn(move |req: Request<Body>| {
        shadow_clone!(config_receiver, config_refresh_sender);
        async move {
            proxy(
                req,
                HttpClient::new(),
                config_receiver.recv().await.expect("receive proxy config"),
                move || {
                    config_refresh_sender.clone().send(()).expect("schedule proxy config refresh");
                }
            ).await
        }
    });

    let make_service = make_service_fn(move |_| {
        shadow_clone!(service);
        async move {
            Ok::<_, Infallible>(service)
        }
    });

    let server = Server::bind(&addr).serve(make_service);
    println!("Listening on http://{}", addr);

    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
}

fn route(uri: &mut http::Uri) {
    *uri = "http://localhost:8000/".parse().unwrap();
}

fn map_request(mut req: Request<Body>) -> Request<Body> {
    route(req.uri_mut());
    req
}

async fn proxy(
    mut req: Request<Body>,
    client: HttpClient,
    proxy_config: ProxyConfig,
    schedule_config_refresh: impl Fn(),
) -> Result<Response<Body>, hyper::Error> {
    schedule_config_refresh();

    println!("req: {:#?}", req);
    println!("proxy_config: {:#?}", proxy_config);

    req = map_request(req);

    if Method::CONNECT == req.method() {
        // Received an HTTP request like:
        // ```
        // CONNECT www.domain.com:443 HTTP/1.1
        // Host: www.domain.com:443
        // Proxy-Connection: Keep-Alive
        // ```
        //
        // When HTTP method is CONNECT we should return an empty body
        // then we can eventually upgrade the connection and talk a new protocol.
        //
        // Note: only after client received an empty body with STATUS_OK can the
        // connection be upgraded, so we can't return a response inside
        // `on_upgrade` future.
        if let Some(addr) = host_addr(req.uri()) {
            task::spawn(async move {
                match req.into_body().on_upgrade().await {
                    Ok(upgraded) => {
                        if let Err(e) = tunnel(upgraded, addr).await {
                            eprintln!("server io error: {}", e);
                        };
                    }
                    Err(e) => eprintln!("upgrade error: {}", e),
                }
            });

            Ok(Response::new(Body::empty()))
        } else {
            eprintln!("CONNECT host is not socket addr: {:?}", req.uri());
            let mut resp = Response::new(Body::from("CONNECT must be to a socket address"));
            *resp.status_mut() = http::StatusCode::BAD_REQUEST;

            Ok(resp)
        }
    } else {
        client.request(req).await
    }
}

fn host_addr(uri: &http::Uri) -> Option<SocketAddr> {
    uri.authority().and_then(|auth| auth.as_str().parse().ok())
}

// Create a TCP connection to host:port, build a tunnel between the connection and
// the upgraded connection
async fn tunnel(upgraded: Upgraded, addr: SocketAddr) -> std::io::Result<()> {
    // Connect to remote server
    let mut server = TcpStream::connect(addr).await?;

    // Proxying data
    let amounts = {
        let (mut server_rd, mut server_wr) = server.split();
        let (mut client_rd, mut client_wr) = tokio::io::split(upgraded);

        let client_to_server = tokio::io::copy(&mut client_rd, &mut server_wr);
        let server_to_client = tokio::io::copy(&mut server_rd, &mut client_wr);

        try_join(client_to_server, server_to_client).await
    };

    // Print message when done
    match amounts {
        Ok((from_client, from_server)) => {
            println!(
                "client wrote {} bytes and received {} bytes",
                from_client, from_server
            );
        }
        Err(e) => {
            println!("tunnel error: {}", e);
        }
    };
    Ok(())
}
