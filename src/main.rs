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
            connector.set_read_timeout(Some(Duration::from_secs(u64::from(proxy_config.timeout))));
            Client::builder().build(connector)
        },
        on_request
    ).start().await
}

