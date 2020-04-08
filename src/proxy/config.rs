use std::net::SocketAddr;
use std::path::Path;
use serde_derive::Deserialize;
use toml;
use tokio::fs;
use http::Uri;

// ------ ProxyConfig ------

/// Proxy configuration loaded from the TOML file.
#[derive(Debug, Deserialize, Clone)]
pub struct ProxyConfig {
    /// Send a request with this url path to schedule reload of this configuration
    ///
    /// (e.g. GET http://example.com/url/path/for/reloading).
    ///
    /// # Example (TOML)
    ///
    /// ```toml
    /// reload_config_url_path = "/reload-proxy-config"
    /// ```
    pub reload_config_url_path: String,

    /// Where the cached responses should be saved.
    ///
    /// # Example (TOML)
    ///
    /// ```toml
    /// cache_file_path = "proxy_cache"
    /// ```
    pub cache_file_path: String,

    /// The address of the new proxy server.
    ///
    /// # Example (TOML)
    ///
    /// ```toml
    /// socket_address = "127.0.0.1:8100"
    /// ```
    pub socket_address: SocketAddr,

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

