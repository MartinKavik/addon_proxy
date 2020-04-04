use ::addon_proxy::{proxy::Proxy, proxy_request};
use hyper::Client;

#[tokio::main]
async fn main() {
    Proxy::new(Client::new()).start(proxy_request).await
}
