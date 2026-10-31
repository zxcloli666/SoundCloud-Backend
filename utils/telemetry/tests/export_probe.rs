mod probe;

use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use probe::load::{self, Sent};

const CONNECTIONS: u64 = 64;
const REQUESTS_PER_CONNECTION: u64 = 100;

fn runtime(threads: usize) -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(threads)
        .enable_all()
        .build()
        .expect("runtime")
}

#[test]
#[ignore = "needs OTEL_EXPORTER_OTLP_ENDPOINT and PROBE_JAEGER of ops/jaeger/probe/run.sh"]
fn every_request_span_reaches_jaeger_under_concurrent_load() {
    let jaeger: SocketAddr = std::env::var("PROBE_JAEGER")
        .expect("PROBE_JAEGER")
        .parse()
        .expect("PROBE_JAEGER is host:port");
    let server = runtime(4);
    let guard = server.block_on(async {
        telemetry::init("api", "info,sqlx=warn", &["export_probe", "telemetry"])
    });
    let listener = server
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let storage = server
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .expect("bind storage");
    probe::app::STORAGE
        .set(storage.local_addr().expect("storage addr"))
        .expect("storage address set once");
    server.spawn(async move { axum::serve(storage, probe::app::storage()).await });
    server.spawn(async move { axum::serve(listener, probe::app::build()).await });

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    let client = runtime(2);
    let started = Instant::now();
    let sent = client.block_on(async {
        let mut tasks = Vec::new();
        for index in 0..CONNECTIONS {
            let first = index * REQUESTS_PER_CONNECTION;
            tasks.push(tokio::spawn(load::connection(
                addr,
                stamp,
                first,
                REQUESTS_PER_CONNECTION,
                Duration::from_millis(200),
            )));
        }
        let mut sent = Sent::default();
        for task in tasks {
            sent.add(task.await.expect("connection"));
        }
        sent
    });
    let load = started.elapsed().as_secs_f64();
    drop(guard);
    println!("load took {load:.1} s, exporter flushed");
    std::thread::sleep(Duration::from_secs(25));

    let found = client.block_on(load::kept(jaeger, &sent.debug));
    println!(
        "created {} spans ({} requests, {} nested), debug traces kept {found} of {}",
        sent.requests + sent.nested,
        sent.requests,
        sent.nested,
        sent.debug.len()
    );
    assert!(
        found * 100 >= sent.debug.len() * 98,
        "only {found} of {} debug traces reached Jaeger",
        sent.debug.len()
    );
    let hop = sent
        .hops
        .first()
        .expect("a debug request went through the hop");
    let spans = client.block_on(load::spans_of(jaeger, hop));
    assert!(
        spans >= 3,
        "the hop trace should hold the api root, the storage call and the storage root, saw {spans}"
    );
}
