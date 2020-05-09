// It's needed at least on stable Rust (1.42.0). Nightly (1.44.0) works without it.
#![type_length_limit="1800000"]  // default is 1048576

use std::time::Duration;
use hyper::Client;
use hyper_tls::HttpsConnector;
use hyper_timeout::TimeoutConnector;
use ::addon_proxy::{Proxy, on_request};

#[tokio::main]
async fn main() {
    Proxy::new(
        |proxy_config| {
            let https = HttpsConnector::new();
            let mut connector = TimeoutConnector::new(https);
            connector.set_read_timeout(Some(Duration::from_secs(proxy_config.timeout)));
            Client::builder().build(connector)
        },
        on_request
    ).start().await
}

