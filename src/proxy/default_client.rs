use super::ProxyConfig;

use std::time::Duration;

use hyper::Client;
use hyper::client::HttpConnector;
use hyper_timeout::TimeoutConnector;
use hyper_tls::HttpsConnector;

pub fn default_client(proxy_config: &ProxyConfig) -> Client<TimeoutConnector<HttpsConnector<HttpConnector>>> {
    let https = HttpsConnector::new();
    let mut connector = TimeoutConnector::new(https);
    connector.set_read_timeout(Some(Duration::from_secs(u64::from(
        proxy_config.timeout,
    ))));
    Client::builder().build(connector)
}
