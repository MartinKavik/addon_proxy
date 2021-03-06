use http::Uri;
use serde_derive::Deserialize;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use tokio::fs;

// ------ ProxyConfig ------

/// Proxy configuration loaded from the TOML file.
#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Deserialize, Clone)]
pub struct ProxyConfig {
    /// Send a request with this url path to schedule reload of this configuration.
    ///
    /// (e.g. GET http://example.com/url/path/for/reloading).
    ///
    /// # Example (TOML)
    ///
    /// ```toml
    /// reload_config_url_path = "/reload-proxy-config"
    /// ```
    pub reload_config_url_path: String,

    /// Send a request with this url path to clear cache.
    ///
    /// (e.g. GET http://example.com/url/path/to/clear/cache).
    ///
    /// # Example (TOML)
    ///
    /// ```toml
    /// clear_cache_url_path = "/clear-cache"
    /// ```
    pub clear_cache_url_path: String,

    /// Send a request with this url path to check proxy status.
    ///
    /// (e.g. GET http://example.com/url/path/to/status).
    ///
    /// # Example (TOML)
    ///
    /// ```toml
    /// status_url_path = "/status"
    /// ```
    pub status_url_path: String,

    /// The directory where the cached responses and other proxy data should be saved.
    ///
    /// _Note:_ The directory will be created if does not exists.
    ///
    /// # Example (TOML)
    ///
    /// ```toml
    /// db_directory = "proxy_db"
    /// ```
    pub db_directory: PathBuf,

    /// Proxy server will be listening on this IP (v4 or v6).
    ///
    /// # Example (TOML)
    ///
    /// ```toml
    /// ip = "0.0.0.0"
    /// ```
    pub ip: IpAddr,

    /// Proxy server will be listening on this port
    /// if a value from the environment variable `PORT` cannot be used.
    ///
    /// # Example (TOML)
    ///
    /// ```toml
    /// default_port = 5000
    /// ```
    pub default_port: u16,

    /// Allow to cache responses and load the cached ones.
    ///
    /// # Example (TOML)
    ///
    /// ```toml
    /// cache_enabled = false
    /// ```
    pub cache_enabled: bool,

    /// How many seconds is a cached response valid,
    /// if its validity isn't explicitly defined by its response headers.
    ///
    /// # Example (TOML)
    ///
    /// ```toml
    /// default_cache_validity = 600  # 10 * 60
    /// ```
    pub default_cache_validity: u32,

    /// If the origin is failing for some reason (returning non-200, timing out),
    /// the proxy tries to return the cached response, even if it's stale.
    ///
    /// However we shouldn't return too old response -
    /// older than the number of seconds defined in `cache_stale_threshold_on_fails`.
    ///
    /// # Example (TOML)
    ///
    /// ```toml
    /// cache_stale_threshold_on_fail = 172_800 # 48 * 60 * 60
    /// ```
    pub cache_stale_threshold_on_fail: u32,

    /// How many seconds to wait for the response from origins.
    ///
    /// # Example (TOML)
    ///
    /// ```toml
    /// timeout = 20
    /// ```
    pub timeout: u32,

    /// Routes for the proxy router.
    ///
    /// # Example (TOML)
    ///
    /// ```toml
    /// [[routes]]
    /// from = "sub.domain.com"
    /// to = "http://localhost:8080"
    ///
    /// [[routes]]
    /// from = "dont-validate.com"
    /// to = "http://localhost:8080"
    /// validate = false
    /// ```
    pub routes: Vec<ProxyRoute>,

    /// If `true`, proxy will call some `println!`s with info about
    /// incoming requests, responses, etc.
    ///
    /// It's useful for debugging but it causes a big performance penalty.   
    ///
    /// # Example (TOML)
    ///
    /// ```toml
    /// verbose = false
    /// ```
    pub verbose: bool,
}

impl ProxyConfig {
    /// Read configuration from the TOML file and try to parse it into `ProxyConfig`.
    ///
    /// # Errors
    ///
    /// Returns `String` error when reading the file fails or when TOML parsing fails.
    pub async fn load(path: impl AsRef<Path> + Send) -> Result<Self, String> {
        let config = fs::read_to_string(path)
            .await
            .map_err(|err| err.to_string())?;
        toml::from_str(&config).map_err(|err| err.to_string())
    }
}

// ------ ProxyRoute ------

/// Route for the proxy router.
///
/// # Example (TOML)
///
/// ```toml
/// [[routes]]
/// from = "sub.domain.com"
/// to = "http://localhost:8080"
///
/// [[routes]]
/// from = "dont-validate.com"
/// to = "http://localhost:8080"
/// validate = false
/// ```
#[derive(Debug, Deserialize, Clone)]
pub struct ProxyRoute {
    pub from: String,
    #[serde(with = "http_serde::uri")]
    pub to: Uri,
    pub validate: Option<bool>,
}
