use std::net::SocketAddr;
use serde_derive::Deserialize;
use toml;
use tokio::fs;

const CONFIG_FILE_NAME: &str = "proxy_config.toml";

// ------ ProxyConfig ------

#[derive(Debug, Deserialize, Clone)]
pub struct ProxyConfig {
    pub refresh_config_url_path: String,
    pub cache_file_path: String,
    pub socket_address: SocketAddr,
    pub routes: Vec<ProxyRoute>,
}

impl ProxyConfig {
    pub async fn load() -> ProxyConfig {
        let config = fs::read_to_string(CONFIG_FILE_NAME).await.expect("read proxy config");
        toml::from_str(&config).expect("parse proxy config")
    }
}

// ------ ProxyRoute ------

#[derive(Debug, Deserialize, Clone)]
pub struct ProxyRoute {
    pub from: String,
    pub to: String,
    pub validate: Option<bool>,
}

