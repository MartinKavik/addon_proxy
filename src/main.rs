use ::addon_proxy::{proxy::Proxy, proxy_request};

#[tokio::main]
async fn main() {
    Proxy::start(proxy_request).await
}
