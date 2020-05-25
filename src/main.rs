use ::addon_proxy::{on_request, Proxy};
use hyper::Client;
use hyper_timeout::TimeoutConnector;
use hyper_tls::HttpsConnector;
use std::time::Duration;

#[tokio::main]
async fn main() {
    Proxy::new(
        |proxy_config| {
            let https = HttpsConnector::new();
            let mut connector = TimeoutConnector::new(https);
            connector.set_read_timeout(Some(Duration::from_secs(u64::from(proxy_config.timeout))));
            Client::builder().build(connector)
        },
        on_request,
    )
    .start()
    .await
}
