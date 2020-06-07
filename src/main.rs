use ::addon_proxy::{on_request, Proxy, default_client};

#[tokio::main]
async fn main() {
    Proxy::new(
        default_client,
        on_request,
    )
    .start()
    .await
}
