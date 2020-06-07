use test_framework::test_callbacks;

#[test_callbacks]
#[cfg(test)]
mod routing {
    use once_cell::sync::Lazy;

    use std::sync::mpsc;
    use std::sync::Mutex;

    use http_test_server::TestServer;

    use ::addon_proxy::{default_client, on_request, Proxy};
    use hyper::{Client, StatusCode, Uri};

    static PROXY_STOPPER: Lazy<Mutex<Option<Box<dyn FnOnce() + Send>>>> =
        Lazy::new(|| Mutex::new(None));
    static MOCK_SERVER: Lazy<Mutex<Option<TestServer>>> = Lazy::new(|| Mutex::new(None));

    // ------ SETUP ------

    fn before_all() {
        let proxy_stopper = start_proxy("test_data/proxy_cfg_no_cache.toml");
        *PROXY_STOPPER.lock().unwrap() = Some(Box::new(proxy_stopper));
        *MOCK_SERVER.lock().unwrap() = Some(start_mock_server());
    }

    fn before_each() {}

    fn after_each() {}

    fn after_all() {
        PROXY_STOPPER.lock().unwrap().take().unwrap()();
        MOCK_SERVER.lock().unwrap().take().unwrap();
    }

    // ------ TESTS ------

    #[tokio::test]
    async fn status() {
        let path = "/status";
        let res = Client::new().get(url_from_path(path)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK,);
    }

    #[tokio::test]
    async fn manifest() {
        let path = "/origin/manifest.json";
        let res = Client::new().get(url_from_path(path)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK,);
    }

    #[tokio::test]
    async fn top() {
        let path = "/origin/catalog/movie/top.json";
        let res = Client::new().get(url_from_path(path)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK,);
    }

    // ------ SETUP HELPERS ------

    fn start_proxy(config_path: &'static str) -> impl FnOnce() {
        let (controller_sender, controller_receiver) = mpsc::channel();
        let (stop_signal_sender, stop_signal_receiver) = mpsc::channel();

        std::thread::spawn(move || {
            let proxy = async {
                Proxy::new(default_client, on_request)
                    .set_config_path(config_path)
                    .set_on_server_start(move |controller| {
                        controller_sender
                            .send(controller)
                            .expect("send proxy controller")
                    })
                    .set_on_server_stop(move || {
                        stop_signal_sender.send(()).expect("send stop signal")
                    })
                    .start()
                    .await
            };

            let mut rt = tokio::runtime::Builder::new()
                .enable_all()
                .basic_scheduler()
                .build()
                .expect("rt build");

            rt.block_on(proxy)
        });

        let controller = controller_receiver.recv().expect("receive proxy ctrl");
        move || {
            controller.stop();
            stop_signal_receiver.recv().expect("receive stop signal");
        }
    }

    #[must_use = "Mock server is stopped on drop"]
    fn start_mock_server() -> TestServer {
        let mock_server = TestServer::new_with_port(5005).unwrap();

        mock_server
            .create_resource("/manifest.json")
            .header("Content-Type", "application/json")
            .body(include_str!("../test_data/manifest.json"));

        mock_server
            .create_resource("/catalog/movie/top.json")
            .header("Content-Type", "application/json")
            .body(include_str!("../test_data/top.json"));

        mock_server
    }

    fn url_from_path(path: &str) -> Uri {
        let proxy_url = "http://127.0.0.1:5000";
        format!("{}{}", proxy_url, path).parse().unwrap()
    }
}
