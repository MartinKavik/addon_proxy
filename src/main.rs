#![type_length_limit="1155333"]  // default is 1048576

use ::addon_proxy::{proxy::Proxy, on_request};
use hyper::Client;

#[tokio::main]
async fn main() {
    Proxy::new(Client::new(), on_request).start().await
}
