use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use hyper::service::{make_service_fn, service_fn};
use hyper::upgrade::Upgraded;
use hyper::{Body, Client, Method, Request, Response, Server};

use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::task;

mod config;

pub use config::{ProxyConfig, ProxyRoute};

pub type HttpClient = Client<hyper::client::HttpConnector>;

macro_rules! shadow_clone {
    ($ ($to_clone:ident) ,*) => {
        $(
            #[allow(unused_mut)]
            let mut $to_clone = $to_clone.clone();
        )*
    };
}

// ------ Proxy ------

pub struct Proxy;

impl Proxy {
    pub async fn start<PR, PRO>(proxy_request: PR)
        where
            PRO: Future<Output = Result<Response<Body>, hyper::Error>> + Send,
            PR: Fn(Request<Body>, HttpClient, Arc<ProxyConfig>, Box<dyn Fn() + Send>) -> PRO + Send + Sync + Copy + 'static
    {
        // @TODO init db?
        // https://github.com/TheNeikos/rustbreak
        // https://github.com/spacejam/sled

        let proxy_config = ProxyConfig::load().await.expect("load proxy config");
        let addr = proxy_config.socket_address.clone();

        let (config_reload_sender, mut config_reload_receiver) = mpsc::unbounded_channel();
        let (config_sender, config_receiver) = watch::channel(Arc::new(proxy_config));

        task::spawn(async move {
            while let Some(_) = config_reload_receiver.recv().await {
                match ProxyConfig::load().await {
                    Ok(proxy_config) => {
                        config_sender.broadcast(Arc::new(proxy_config)).expect("broadcast reloaded config");
                        println!("proxy config reloaded");
                    },
                    Err(err) => eprintln!("cannot reload proxy config: {}", err)
                }
            }
        });

        let service = service_fn(move |req: Request<Body>| {
            shadow_clone!(config_receiver, config_reload_sender);
            async move {
                proxy_request(
                    req,
                    HttpClient::new(),
                    config_receiver.recv().await.expect("receive proxy config"),
                    Box::new(move || {
                        config_reload_sender.clone().send(()).expect("schedule proxy config reload");
                    })
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
}
