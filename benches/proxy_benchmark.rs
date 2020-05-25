use futures::future::join_all;
use std::iter;
use std::time::{Instant, Duration};
use std::rc::Rc;
use std::cell::RefCell;
use std::sync::mpsc;

use criterion::{criterion_group, criterion_main, Criterion, Bencher, BatchSize};

use http_test_server::TestServer;

use hyper::Client;
use hyper_tls::HttpsConnector;
use hyper_timeout::TimeoutConnector;
use ::addon_proxy::{Proxy, on_request};

type TestData = Rc<RefCell<(Duration, u32)>>;

// @TODO add `cargo bench` to README

// @TODO `cargo make verify`

pub fn criterion_benchmark(c: &mut Criterion) {
    let _mock_server = start_mock_server();
    // NOTE: DNS can be slow, use rather IP.
    let proxy_url = "http://127.0.0.1:5000";   

    // @TODO remove old proxy_db/

    {
        start_proxy("bench_data/proxy_cfg_no_cache.toml");

        proxy_bench(c, proxy_url, "status", 1000, 1, "/status");
        proxy_bench(c, proxy_url, "status_parallel", 10_000, 100, "/status");
        // NOTE: It runs for cca 15 minutes.
        // proxy_bench(c, &mut rt, "status_parallel_long", 1_000_000, 1000, proxy_url + "status");

        // NOTE: Origin is called through `localhost` => 
        // change the route in TOML config to `127.0.0.1` once the issue is resolved:
        // https://github.com/viniciusgerevini/http-test-server/issues/7
        proxy_bench(c, proxy_url, "manifest | no_cache", 100, 1, "/origin/manifest.json");
        proxy_bench(c, proxy_url, "manifest_parallel | no_cache", 1_000, 100, "/origin/manifest.json");
        proxy_bench(c, proxy_url, "top | no_cache", 100, 1, "/origin/catalog/movie/top.json");
        proxy_bench(c, proxy_url, "top_parallel | no_cache", 1_000, 100, "/origin/catalog/movie/top.json");
    }

    // @TODO with cache
}

criterion_group!{
    name = benches;
    config = Criterion::default().sample_size(10).warm_up_time(Duration::new(1, 0));
    targets = criterion_benchmark
}
criterion_main!(benches);

// ------ Start* Helpers ------

fn start_proxy(config_path: &'static str) {
    let (on_start_sender, on_start_receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let proxy = async { 
            Proxy::new(
                |proxy_config| {
                    let https = HttpsConnector::new();
                    let mut connector = TimeoutConnector::new(https);
                    connector.set_read_timeout(Some(Duration::from_secs(u64::from(proxy_config.timeout))));
                    Client::builder().build(connector)
                },
                on_request
            )
                .set_config_path(config_path)
                .set_on_server_start(move || on_start_sender.send(()).expect("send start notification"))
                .start().await
        };

        let mut rt = tokio::runtime::Builder::new()
            .enable_all()
            .basic_scheduler()
            .build()
            .expect("rt build");

        rt.block_on(proxy)
    });
    on_start_receiver.recv().expect("receive start notification");
}

#[must_use = "TestServer is stopped on drop"]
fn start_mock_server() -> TestServer {
    let mock_server = TestServer::new_with_port(5005).unwrap();

    mock_server
        .create_resource("/manifest.json")
        .header("Content-Type", "application/json")
        .body(include_str!("../bench_data/manifest.json"));

    mock_server
        .create_resource("/catalog/movie/top.json")
        .header("Content-Type", "application/json")
        .body(include_str!("../bench_data/top.json"));

    mock_server
}

// ------ Bench Helpers ------

fn proxy_bench(c: &mut Criterion, proxy_url: &str, name: &str, num_of_all_reqs: usize, num_of_users: usize, path: &str) {
    let test_data = Rc::new(RefCell::new((Duration::default(), 0)));

    c.bench_function(name, |b| bench_requests(
        b, num_of_all_reqs, num_of_users, &format!("{}{}", proxy_url, path), &test_data)
    );
    
    let test_data = test_data.borrow();

    println!("_______________________________________________________");
    println!("Bench name ............................... {}", name);
    println!("Number of all requests per iteration...... {}", num_of_all_reqs);
    println!("Number of users .......................... {}", num_of_users);
    println!("Send request & read response avg time .... {:#?}", test_data.0 / test_data.1);
    println!("Number of all requests ................... {}", test_data.1);
    println!("Path ..................................... {}", path);
    println!("_______________________________________________________");
}

fn bench_requests(b: &mut Bencher, num_of_all_requests: usize, users: usize, url: &str, test_data: &TestData) {
    // NOTE: We want to create a fresh `Runtime` to quickly kill the old connections.
    let mut rt = tokio::runtime::Builder::new()
        .enable_all()
        .basic_scheduler()
        .build()
        .expect("rt build");
    
    let client = hyper::Client::new();

    b.iter_batched(
        || create_requests(url, &client, num_of_all_requests, users, test_data),
        |requests| rt.block_on(requests),
        BatchSize::SmallInput
    );
}

async fn create_requests(
    url: &str, 
    client: &hyper::Client<hyper::client::HttpConnector>, 
    num_of_all_requests: usize, 
    users: usize,
    test_data: &TestData,
) -> () {
    let url: hyper::Uri = url.parse().expect("parsed url");
    let sequence_length = num_of_all_requests / users; 

    join_all(iter::repeat_with(|| {
        async {
            for _ in 0..sequence_length {
                let now = Instant::now();

                let res = client.get(url.clone()).await.expect("get response");
                assert_eq!(res.status(), hyper::StatusCode::OK, "Did not receive a 200 HTTP status code.");
                // Read response body until the end.
                hyper::body::to_bytes(res.into_body()).await.expect("read response body");

                let mut test_data = test_data.borrow_mut();
                test_data.0 += now.elapsed();
                test_data.1 += 1;
            }
        }
    }).take(users).collect::<Vec<_>>()).await;
}
