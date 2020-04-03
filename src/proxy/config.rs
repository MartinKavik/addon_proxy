use std::net::SocketAddr;
use serde_derive::Deserialize;
use toml;
use tokio::fs;

const CONFIG_FILE_NAME: &str = "proxy_config.toml";

// ------ ProxyConfig ------

#[derive(Debug, Deserialize, Clone)]
pub struct ProxyConfig {
    pub reload_config_url_path: String,
    pub cache_file_path: String,
    pub socket_address: SocketAddr,
    pub routes: Vec<ProxyRoute>,
}

impl ProxyConfig {
    pub async fn load() -> Result<ProxyConfig, String> {
        let config = fs::read_to_string(CONFIG_FILE_NAME).await.map_err(|err| err.to_string())?;
        toml::from_str(&config).map_err(|err| err.to_string())
    }
}

// ------ ProxyRoute ------

#[derive(Debug, Deserialize, Clone)]
pub struct ProxyRoute {
    pub from: String,
    pub to: String,
    pub validate: Option<bool>,
}

