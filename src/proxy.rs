use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::env;
use std::net::SocketAddr;

use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Client, Request, Response, Server};

use tokio::sync::{mpsc, watch, oneshot};
use tokio::task;

use sled;
use shadow_clone::shadow_clone;

mod config;
mod controller;
mod on_request;
mod validations;

pub use config::{ProxyConfig, ProxyRoute};
pub use controller::ProxyController;
pub use on_request::on_request;

pub const DEFAULT_CONFIG_PATH: &str = "proxy_config.toml";

// ------ Proxy ------

/// See documentation for `Proxy` field `on_request`.
pub type ScheduleConfigReload = Arc<dyn Fn() + Send + Sync>;
pub type Db = sled::Db;

/// Represents a proxy server.
///
/// See field documentation for more details.
///
/// # Example
///
/// ```rust,no_run
/// use ::addon_proxy::{proxy::Proxy, on_request};
/// use hyper::Client;
///
/// #[tokio::main]
/// async fn main() {
///     Proxy::new(Client::new(), on_request).start().await
/// }
/// ```
///
/// # Type parameters
///
/// - `C` = client connector
/// - `B` = request body
/// - `CC` = client creator
/// - `OR` = `on_request` callback
/// - `ORO` = `on_request` output (aka callback's return value)
pub struct Proxy<C, B, CC, OR, ORO> {
    /// Where the TOML file with settings is located.
    pub config_path: PathBuf,

    /// A function that returns a client that is passed to all `on_request` calls.
    ///
    /// _Note:_ To support also TLS and use other connectors, see
    /// [hyper.rs Client configuration](https://hyper.rs/guides/client/configuration/).
    pub client_creator: CC,

    /// `on_request` is invoked for each request.
    /// It allows you to modify or validate the original request.
    ///
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
    ///    to new requests after the call.
    ///
    /// - `db` - Persistent storage to support features like caching.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use std::sync::Arc;
    /// use hyper::{Body, Client, Request, Response};
    /// use hyper::client::HttpConnector;
    /// use proxy::{ProxyConfig, ScheduleConfigReload, Db};
    ///
    /// pub async fn on_request(
    ///     req: Request<Body>,
    ///     client: Arc<Client<HttpConnector>>,
    ///     proxy_config: Arc<ProxyConfig>,
    ///     schedule_config_reload: ScheduleConfigReload,
    ///     db: Db,
    /// ) -> Result<Response<Body>, hyper::Error> {
    ///     println!("original req: {:#?}", req);
    ///     let req = try_map_request(req, &proxy_config, schedule_config_reload, &db);
    ///     println!("mapped req or response: {:#?}", req);
    ///     match req {
    ///         Ok(req) => client.request(req).await,
    ///         Err(response) => Ok(response)
    ///     }
    /// }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns `hyper::Error` when request fails.
    pub on_request: OR,

    // Callback `on_server_start` is invoked on server start.
    // You can stop the server by calling `ProxyController::stop`.
    pub on_server_start: Option<Box<dyn FnOnce(ProxyController)>>,

    _phantom: (PhantomData<C>, PhantomData<B>, PhantomData<ORO>),
}

impl<C, B, CC, OR, ORO> Proxy<C, B, CC, OR, ORO>
    where
        C: Send + Sync + 'static,
        B: Send + 'static,
        CC: Fn(&ProxyConfig) -> Client<C, B>,
        ORO: Future<Output = Result<Response<Body>, hyper::Error>> + Send,
        OR: Fn(Request<Body>, Arc<Client<C, B>>, Arc<ProxyConfig>, ScheduleConfigReload, Db) -> ORO + Send + Sync + Copy + 'static,
{
    /// Create a new `Proxy` instance.
    ///
    /// # Arguments
    ///
    /// See documentation for struct `Proxy` fields.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ::addon_proxy::{proxy::Proxy, on_request};
    /// use hyper::Client;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     Proxy::new(|_proxy_config| Client::new(), on_request).start().await
    /// }
    /// ```
    pub fn new(client_creator: CC, on_request: OR) -> Self {
        Self {
            config_path: PathBuf::from(DEFAULT_CONFIG_PATH),
            client_creator,
            on_request,
            on_server_start: None,
            _phantom: (PhantomData, PhantomData, PhantomData)
        }
    }

    /// Set proxy config file path.
    ///
    /// Default is `proxy_config.toml`.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ::addon_proxy::{proxy::Proxy, on_request};
    /// use hyper::Client;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     Proxy::new(Client::new(), on_request)
    ///         .set_config_path("proxy_config.toml")
    ///         .start()
    ///         .await
    /// }
    /// ```
    pub fn set_config_path(&mut self, path: impl Into<PathBuf>) -> &mut Self {
        self.config_path = path.into();
        self
    }

    /// Provided callback is invoked on server start.
    ///
    /// It's useful when you have to make sure the server is running - e.g. in benchmarks.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ::addon_proxy::{proxy::Proxy, on_request};
    /// use hyper::Client;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     Proxy::new(Client::new(), on_request)
    ///         .set_on_server_start(|_controller| println!("Server started!"))
    ///         .start()
    ///         .await
    /// }
    /// ```
    pub fn set_on_server_start(&mut self, on_server_start: impl FnOnce(ProxyController) + 'static) -> &mut Self {
        self.on_server_start = Some(Box::new(on_server_start));
        self
    }

    /// Start the `Proxy` server.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use ::addon_proxy::{proxy::Proxy, on_request};
    /// use hyper::Client;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     Proxy::new(Client::new(), on_request).start().await
    /// }
    /// ```
    ///
    /// # Panics
    ///
    /// - Almost immediately after the `start` call
    ///    - If the proxy config loading failed (e.g. TOML file with the configuration cannot be found).
    ///    - If the database opening failed (e.g. the storage directory cannot be created).
    /// - While the server is running and it's not possible to send items through a channel
    /// (this shouldn't happen in practice).
    pub async fn start(&mut self) {
        let on_request = self.on_request;
        let config_path = self.config_path.clone();
        let proxy_config = ProxyConfig::load(&config_path).await.expect("load proxy config");
        let client = Arc::new((&self.client_creator)(&proxy_config));
        let addr = SocketAddr::new(
            proxy_config.ip,
            env::var("PORT").ok().and_then(|port| port.parse().ok())
                .unwrap_or(proxy_config.default_port),
        );
        // All operations in sled are thread-safe.
        // The Db may be cloned and shared across threads without needing to use Arc or Mutex etc…
        let db = sled::open(&proxy_config.db_directory).expect("open database");

        // `config_reload_sender` will be used to schedule proxy config reload from `on_request` callbacks.
        // `config_reload_receiver` will be used in the standalone task to listen for `schedule_config_reload` calls.
        let (config_reload_sender, mut config_reload_receiver) = mpsc::unbounded_channel();
        // `config_sender` will be used to send a (re)loaded config to the request service.
        // `config_receiver` will be used to accept the sent config.
        let (config_sender, config_receiver) = watch::channel(Arc::new(proxy_config));

        // Spawn a new task that broadcasts (re)loaded configs.
        // These configs are picked just before the `on_request` callback is called.
        task::spawn(async move {
            while let Some(_) = config_reload_receiver.recv().await {
                match ProxyConfig::load(&config_path).await {
                    Ok(proxy_config) => {
                        config_sender.broadcast(Arc::new(proxy_config)).expect("broadcast reloaded config");
                        println!("proxy config reloaded");
                    },
                    Err(err) => eprintln!("cannot reload proxy config: {}", err)
                }
            }
        });

        // `schedule_config_reload` will be passed to all `on_request` callbacks.
        let schedule_config_reload = Arc::new(move || {
            config_reload_sender.clone().send(()).expect("schedule proxy config reload");
        });

        // The request service. It's usually bound to a single connection.
        // The callback will be executed for each request.
        let service = service_fn({
            shadow_clone!(db);
            move |req: Request<Body>| {
                shadow_clone!(mut config_receiver, client, schedule_config_reload, db);
                async move {
                    on_request(
                        req,
                        client,
                        config_receiver.recv().await.expect("receive proxy config"),
                        schedule_config_reload,
                        db,
                    ).await
                }
            }
        });

        // Since a request service is bound to a single connection,
        // a server needs a way to make them as it accepts connections.
        // This is what a `make_service_fn` does.
        let make_service = make_service_fn(move |_| {
            shadow_clone!(service);
            async move {
                Ok::<_, Infallible>(service)
            }
        });

        let server = Server::bind(&addr).serve(make_service);
        println!("Listening on http://{}", addr);

        // Prepare controller with ability to gracefully shutdown the server.
        let (shutdown_sender, shutdown_receiver) = oneshot::channel::<()>();
        let server = server.with_graceful_shutdown(async { shutdown_receiver.await.ok(); });

        if let Some(on_server_start) = self.on_server_start.take() {
            on_server_start(ProxyController { shutdown_sender });
        }

        // Block until the server is stopped.
        if let Err(e) = server.await {
            eprintln!("server error: {}", e);
        }

        // Save dirty data.
        if let Err(e) = db.flush_async().await {
            eprintln!("database flush error: {}", e);
        }
    }
}
