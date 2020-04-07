use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;
use std::marker::PhantomData;

use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Client, Request, Response, Server};

use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::task;

mod config;

pub use config::{ProxyConfig, ProxyRoute};

macro_rules! shadow_clone {
    ($ ($to_clone:ident) ,*) => {
        $(
            #[allow(unused_mut)]
            let mut $to_clone = $to_clone.clone();
        )*
    };
}

// ------ Proxy ------

pub struct Proxy<C, B, OR, ORO> {
    pub client: Arc<Client<C, B>>,
    /// `on_request` is invoked for each request.
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
    /// # Arguments
    ///
    /// - `req` - The original request.
    ///
    /// - `client` - The client set in the `Proxy` instance.
    ///
    /// - `proxy_config` - A configuration loaded from `proxy_config.toml`.
    ///
    /// - `schedule_config_reload` - The configuration will be reloaded and passed
    ///    to new requests after call.
    ///
    /// # Example
    /// ```rust,no_run
    /// use std::sync::Arc;
    /// use hyper::{Body, Client, Request, Response};
    /// use hyper::client::HttpConnector;
    /// use proxy::ProxyConfig;
    ///
    /// pub async fn on_request(
    ///     req: Request<Body>,
    ///     client: Arc<Client<HttpConnector>>,
    ///     proxy_config: Arc<ProxyConfig>,
    ///     schedule_config_reload: impl Fn(),
    /// ) -> Result<Response<Body>, hyper::Error> {
    ///     println!("original req: {:#?}", req);
    ///     let req = try_map_request(req, &proxy_config, schedule_config_reload);
    ///     println!("mapped req or response: {:#?}", req);
    ///     match req {
    ///         Ok(req) => client.request(req).await,
    ///         Err(response) => Ok(response)
    ///     }
    /// }
    /// ```
    ///
    /// # Errors
    /// Returns `hyper::Error` when request fails.
    pub on_request: OR,
    _phantom: PhantomData<ORO>,
}

impl<C, B, OR, ORO> Proxy<C, B, OR, ORO>
    where
        C: Send + Sync + 'static,
        B: Send + 'static,
        ORO: Future<Output = Result<Response<Body>, hyper::Error>> + Send,
        OR: Fn(Request<Body>, Arc<Client<C, B>>, Arc<ProxyConfig>, Box<dyn Fn() + Send>) -> ORO + Send + Sync + Copy + 'static,
{
    pub fn new(client: Client<C, B>, on_request: OR) -> Self {
        Self {
            client: Arc::new(client),
            on_request,
            _phantom: PhantomData
        }
    }

    pub async fn start(&self) {
        // @TODO init db
        // https://github.com/TheNeikos/rustbreak
        // https://github.com/spacejam/sled

        let on_request = self.on_request;
        let client = Arc::clone(&self.client);
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
            shadow_clone!(config_receiver, config_reload_sender, client);
            async move {
                on_request(
                    req,
                    client,
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
