use test_framework::test_callbacks;

#[test_callbacks]
#[cfg(test)]
mod caching {
    use once_cell::sync::Lazy;

    use std::sync::mpsc;
    use std::sync::Mutex;

    use http_test_server::TestServer;
    use chrono::Utc;

    use ::addon_proxy::{default_client, on_request, Proxy, helpers::set_now_getter};
    use hyper::{Client, StatusCode, Uri};
    use hyper::client::HttpConnector;

    static PROXY_STOPPER: Lazy<Mutex<Option<Box<dyn FnOnce() + Send>>>> =
        Lazy::new(|| Mutex::new(None));

    // ------ SETUP ------

    fn before_all() {
        let proxy_stopper = start_proxy("test_data/proxy_cfg.toml");
        *PROXY_STOPPER.lock().unwrap() = Some(Box::new(proxy_stopper));
    }

    fn before_each() {}

    fn after_each() {}

    fn after_all() {
        PROXY_STOPPER.lock().unwrap().take().unwrap()();
    }

    // ------ TESTS ------

    // Caching: the proxy must cache responses by respecting the HTTP cache headers that come from the origin


    #[tokio::test]
    async fn caching_test_suite() {
        let client = Client::new();
        // Run tests sequentially because we need to manipulate with time and modify cache. 
        test_no_headers(&client).await;
        test_max_age_header(&client).await;
        test_stale_response(&client).await;
    }

    // "If no cache headers are returned at all, assume 10 minutes cache validity."
    async fn test_no_headers(client: &Client<HttpConnector>) {
        // ------ ARRANGE ------
        clear_cache().await;
        set_now_getter(|| Utc::now().timestamp());

        let mock_server = start_mock_server();
        let resource = mock_server.create_resource("/catalog/movie/top.json");
        resource
            .body(include_str!("../test_data/top.json"));

        let path = "/origin/catalog/movie/top.json";
        let send_request = || async { client.get(url_from_path(path)).await.unwrap() };

        // ------ ACT ------

        send_request().await;  // This request should be loaded from the origin.
        send_request().await;

        // Jump in time: +11 min. Jump has to be longer than `default_cache_validity` in proxy config.       
        set_now_getter(|| Utc::now().timestamp() + (11 * 60)); 

        send_request().await; // This request should be loaded from the origin.
        send_request().await;
        send_request().await;

        // ------ ASSERT ------
        
        // 3 requests should be loaded from cache, 2 from the origin.
        assert_eq!(resource.request_count(), 2);
    }

    // "No need to handle all HTTP cache header specs: just `cache-control` `max-age`"
    async fn test_max_age_header(client: &Client<HttpConnector>) {
        // ------ ARRANGE ------
        clear_cache().await;
        set_now_getter(|| Utc::now().timestamp());

        let mock_server = start_mock_server();
        let resource = mock_server.create_resource("/catalog/movie/top.json");
        resource
            // 300s = 5 min
            .header("Cache-Control", "max-age=300")
            .body(include_str!("../test_data/top.json"));

        let path = "/origin/catalog/movie/top.json";
        let send_request = || async { client.get(url_from_path(path)).await.unwrap() };

        // ------ ACT ------

        send_request().await;  // This request should be loaded from the origin.
        send_request().await;

        // Jump in time: +6 min. Jump has to be longer than `max-age` in the request header.       
        set_now_getter(|| Utc::now().timestamp() + (6 * 60)); 

        send_request().await; // This request should be loaded from the origin.
        send_request().await;
        send_request().await;

        // ------ ASSERT ------
        
        // 3 requests should be loaded from cache, 2 from the origin.
        assert_eq!(resource.request_count(), 2);
    }

    // if the addon origin is failing for some reason (returning non-200, timing out), return the last cached response, even if it's stale*
    // * - there should be a configurable "stale threshold": e.g., only do this if the cached response is not older than 48 hours
    async fn test_stale_response(client: &Client<HttpConnector>) {
        // ------ ARRANGE ------
        clear_cache().await;
        set_now_getter(|| Utc::now().timestamp());

        let mock_server = start_mock_server();
        let resource = mock_server.create_resource("/catalog/movie/top.json");
        resource
            .body(include_str!("../test_data/top.json"));

        let path = "/origin/catalog/movie/top.json";
        let send_request = || async { client.get(url_from_path(path)).await.unwrap() };

        // ------ ACT ------

        assert_eq!(send_request().await.status(), StatusCode::OK);  // This request should be loaded from the origin.
        assert_eq!(send_request().await.status(), StatusCode::OK);

        // Kill `mock_server` to simulate addon fail.
        drop(mock_server);

        assert_eq!(send_request().await.status(), StatusCode::OK);
        assert_eq!(send_request().await.status(), StatusCode::OK);

        // Jump in time: +49 h. Jump has to be longer than `cache_stale_threshold_on_fail` in proxy config.      
        set_now_getter(|| Utc::now().timestamp() + (49 * 60 * 60)); 

        // ------ ASSERT ------
        
        assert_eq!(send_request().await.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(send_request().await.status(), StatusCode::INTERNAL_SERVER_ERROR);
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
        TestServer::new_with_port(5005).unwrap()
    }

    fn url_from_path(path: &str) -> Uri {
        let proxy_url = "http://127.0.0.1:5000";
        format!("{}{}", proxy_url, path).parse().unwrap()
    }

    async fn clear_cache() {
        Client::new().get(url_from_path("/clear-cache")).await.unwrap();
    }
}
