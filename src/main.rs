// It's needed at least on stable Rust (1.42.0). Nightly (1.44.0) works without it.
#![type_length_limit="1800000"]  // default is 1048576

use ::addon_proxy::{Proxy, on_request};
use hyper::Client;
use hyper_tls::HttpsConnector;

#[tokio::main]
async fn main() {
    Proxy::new(
        Client::builder().build(HttpsConnector::new()),
        on_request
    ).start().await
}

