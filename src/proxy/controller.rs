use tokio::sync::oneshot;
/// `ProxyController` is passed to the callback registered by `Proxy::set_on_server_start`.
#[allow(clippy::module_name_repetitions)]
pub struct ProxyController {
    pub(crate) shutdown_sender: oneshot::Sender<()>,
}

impl ProxyController {
    /// Send shutdown signal to the proxy. It's non-blocking.
    ///
    /// You can register your callback by `Proxy::set_on_server_stop` to find out
    /// when the proxy is stopped and its resources have been freed.
    pub fn stop(self) {
        self.shutdown_sender.send(()).expect("send shutdown signal");
    }
}
