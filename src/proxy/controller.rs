use tokio::sync::oneshot;
pub struct ProxyController {
    pub(crate) shutdown_sender: oneshot::Sender<()>
}

impl ProxyController {
    pub fn stop(self) {
        self.shutdown_sender.send(()).expect("send shutdown signal");
    }
}
