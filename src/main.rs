use ::addon_proxy::{default_client, on_request, Proxy};

#[tokio::main]
async fn main() {
    Proxy::new(default_client, on_request).start().await
}
