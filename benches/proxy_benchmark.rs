use futures::future::join_all;
use std::cell::RefCell;
use std::iter;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use criterion::{criterion_group, criterion_main, BatchSize, Bencher, Criterion};

use http_test_server::TestServer;
use remove_dir_all::remove_dir_all;
use separator::Separatable;

use ::addon_proxy::{on_request, Proxy};
use hyper::Client;
use hyper_timeout::TimeoutConnector;
use hyper_tls::HttpsConnector;

#[derive(Default)]
struct BenchData {
    // The sum of all measurements of sending a request and reading the entire response.
    measurements_sum: Duration,
    // The number of all requests.
    requests: u32,
    // Bench time except the setup time.
    time: Duration,
}

#[rustfmt::skip]
pub fn criterion_benchmark(c: &mut Criterion) {
    let _mock_server = start_mock_server();
    // NOTE: DNS can be slow, use rather IP.
    let proxy_url = "http://127.0.0.1:5000";   

    let proxy_db_path = "bench_data/proxy_db";
    if Path::new(proxy_db_path).is_dir() {
        remove_dir_all(proxy_db_path).expect("remove proxy_db directory");
        println!("{} removed.", proxy_db_path);
    }

    // ------ Cache Disabled ------

    let proxy_stopper = start_proxy("bench_data/proxy_cfg_no_cache.toml");
    {
        proxy_bench(c, proxy_url, "status", 1000, 1, "/status");
        proxy_bench(c, proxy_url, "status_parallel", 10_000, 100, "/status");
        proxy_bench(c, proxy_url, "manifest | no_cache", 100, 1, "/origin/manifest.json");
        proxy_bench(c, proxy_url, "manifest_parallel | no_cache", 1_000, 100, "/origin/manifest.json");
        proxy_bench(c, proxy_url, "top | no_cache", 100, 1, "/origin/catalog/movie/top.json");
        proxy_bench(c, proxy_url, "top_parallel | no_cache", 1_000, 100, "/origin/catalog/movie/top.json");
    }
    proxy_stopper();

    // ------ Cache Enabled ------
    
    let proxy_stopper = start_proxy("bench_data/proxy_cfg.toml");
    {
        // NOTE: First requests are NOT cached.
        proxy_bench(c, proxy_url, "manifest", 100, 1, "/origin/manifest.json");
        proxy_bench(c, proxy_url, "manifest_parallel", 1_000, 100, "/origin/manifest.json");
        proxy_bench(c, proxy_url, "top", 100, 1, "/origin/catalog/movie/top.json");
        proxy_bench(c, proxy_url, "top_parallel", 1_000, 100, "/origin/catalog/movie/top.json");
        // NOTE: It runs for cca 15 minutes.
        // proxy_bench(c, proxy_url, "manifest_parallel_long", 1_000_000, 1000, "/origin/manifest.json");
    }
    proxy_stopper();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10).warm_up_time(Duration::new(1, 0));
    targets = criterion_benchmark
}
criterion_main!(benches);

// ------ Start* Helpers ------

fn start_proxy(config_path: &'static str) -> impl FnOnce() {
    let (controller_sender, controller_receiver) = mpsc::channel();
    let (stop_signal_sender, stop_signal_receiver) = mpsc::channel();

    std::thread::spawn(move || {
        let proxy = async {
            Proxy::new(
                |proxy_config| {
                    let https = HttpsConnector::new();
                    let mut connector = TimeoutConnector::new(https);
                    connector.set_read_timeout(Some(Duration::from_secs(u64::from(
                        proxy_config.timeout,
                    ))));
                    Client::builder().build(connector)
                },
                on_request,
            )
            .set_config_path(config_path)
            .set_on_server_start(move |controller| {
                controller_sender
                    .send(controller)
                    .expect("send proxy controller")
            })
            .set_on_server_stop(move || stop_signal_sender.send(()).expect("send stop signal"))
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
        .body(include_str!("../bench_data/manifest.json"));

    mock_server
        .create_resource("/catalog/movie/top.json")
        .header("Content-Type", "application/json")
        .body(include_str!("../bench_data/top.json"));

    mock_server
}

// ------ Bench Helpers ------

#[rustfmt::skip]
fn proxy_bench(c: &mut Criterion, proxy_url: &str, name: &str, num_of_all_reqs: usize, num_of_users: usize, path: &str) {
    let bench_data = Rc::new(RefCell::new(BenchData::default()));

    c.bench_function(name, |b| bench_requests(
        b, num_of_all_reqs, num_of_users, &format!("{}{}", proxy_url, path), &bench_data)
    );
    
    let bench_data = bench_data.borrow();

    let rps = Duration::new(1, 0).as_nanos() / (bench_data.time / bench_data.requests).as_nanos();
    println!("_______________________________________________________");
    println!("Bench name ............................... {}", name);
    println!("Number of all requests per iteration...... {}", num_of_all_reqs.separated_string());
    println!("Number of users .......................... {}", num_of_users.separated_string());
    println!("Send request & read response avg time .... {:#?}", bench_data.measurements_sum / bench_data.requests);
    println!("Requests & readings per second ........... {}", rps.separated_string());
    println!("Number of all requests ................... {}", bench_data.requests.separated_string());
    println!("Bench time ............................... {:#?}", bench_data.time);
    println!("Path ..................................... {}", path);
    println!("_______________________________________________________");
}

fn bench_requests(
    b: &mut Bencher,
    num_of_all_requests: usize,
    users: usize,
    url: &str,
    bench_data: &Rc<RefCell<BenchData>>,
) {
    // NOTE: We want to create a fresh `Runtime` to quickly kill the old connections.
    let mut rt = tokio::runtime::Builder::new()
        .enable_all()
        .basic_scheduler()
        .build()
        .expect("rt build");

    let client = hyper::Client::new();

    b.iter_batched(
        || create_requests(url, &client, num_of_all_requests, users, bench_data),
        |requests| rt.block_on(requests),
        BatchSize::SmallInput,
    );
}

async fn create_requests(
    url: &str,
    client: &hyper::Client<hyper::client::HttpConnector>,
    num_of_all_requests: usize,
    users: usize,
    bench_data: &Rc<RefCell<BenchData>>,
) {
    let url: hyper::Uri = url.parse().expect("parsed url");
    let sequence_length = num_of_all_requests / users;

    let now_for_bench_time = Instant::now();

    join_all(
        iter::repeat_with(|| {
            async {
                for _ in 0..sequence_length {
                    let now = Instant::now();

                    let res = client.get(url.clone()).await.expect("get response");
                    assert_eq!(
                        res.status(),
                        hyper::StatusCode::OK,
                        "Did not receive a 200 HTTP status code."
                    );
                    // Read response body until the end.
                    hyper::body::to_bytes(res.into_body())
                        .await
                        .expect("read response body");

                    let mut bench_data = bench_data.borrow_mut();
                    bench_data.measurements_sum += now.elapsed();
                    bench_data.requests += 1;
                }
            }
        })
        .take(users)
        .collect::<Vec<_>>(),
    )
    .await;

    bench_data.borrow_mut().time += now_for_bench_time.elapsed();
}
