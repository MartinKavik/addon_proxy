use std::net::IpAddr;
use std::path::{Path, PathBuf};
use serde_derive::Deserialize;
use toml;
use tokio::fs;
use http::Uri;

// ------ ProxyConfig ------

/// Proxy configuration loaded from the TOML file.
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

    /// How many seconds to wait for the response from origins.
    ///
    /// # Example (TOML)
    ///
    /// ```toml
    /// timeout = 20
    /// ```
    pub timeout: u64,

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
}

impl ProxyConfig {
    /// Read configuration from the TOML file and try to parse it into `ProxyConfig`.
    pub async fn load(path: impl AsRef<Path>) -> Result<ProxyConfig, String> {
        let config = fs::read_to_string(path).await.map_err(|err| err.to_string())?;
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

