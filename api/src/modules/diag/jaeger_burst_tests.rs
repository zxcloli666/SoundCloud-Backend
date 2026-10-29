use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::routing::get;
use rand::Rng;
use tower::ServiceExt;

use super::jaeger;

const DEBUG_REQUESTS: usize = 3000;
const PLAIN_REQUESTS: usize = 3000;
const ONE_TRACE_REQUESTS: usize = 20_000;
const ONE_TRACE: &str = "deadbeefdeadbeefdeadbeefdeadbeef";

fn hex(len: usize) -> String {
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| char::from(b"0123456789abcdef"[rng.gen_range(0..16)]))
        .collect()
}

fn setting(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

async fn send(app: &Router, trace: &str, flags: &str) {
    let request = Request::get("/health")
        .header("traceparent", format!("00-{trace}-{}-{flags}", hex(16)))
        .body(Body::empty())
        .expect("request");
    let _ = app.clone().oneshot(request).await;
}

async fn kept(http: &wreq::Client, base: &str, ids: &[String]) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for id in ids {
        if let Ok(Some(_)) = jaeger::fetch(http, base, id).await {
            found.insert(id.clone());
        }
    }
    found
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the two Jaeger containers of ops/jaeger/probe/run.sh"]
async fn a_client_debug_flag_keeps_traces_only_up_to_the_cap_and_both_hosts_agree() {
    let a = setting("PROBE_JAEGER_A").expect("PROBE_JAEGER_A");
    let b = setting("PROBE_JAEGER_B").expect("PROBE_JAEGER_B");
    let cap: f64 = setting("TELEMETRY_SPANS_PER_SECOND")
        .and_then(|value| value.parse().ok())
        .unwrap_or(10.0);
    let guard = telemetry::init("api", "warn", &["telemetry"]);
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn(telemetry::trace_requests));

    let started = Instant::now();
    let mut debug_ids = Vec::new();
    let mut plain_ids = Vec::new();
    for index in 0..DEBUG_REQUESTS + PLAIN_REQUESTS {
        let id = hex(32);
        if index % 2 == 0 && debug_ids.len() < DEBUG_REQUESTS {
            send(&app, &id, "01").await;
            debug_ids.push(id);
        } else {
            send(&app, &id, "00").await;
            plain_ids.push(id);
        }
        if index % 200 == 0 {
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
    }
    for index in 0..ONE_TRACE_REQUESTS {
        send(&app, ONE_TRACE, "01").await;
        if index % 200 == 0 {
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
    }
    drop(guard);
    tokio::time::sleep(Duration::from_secs(40)).await;
    let seconds = started.elapsed().as_secs_f64();

    let http = wreq::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("client");
    let mut every_id = debug_ids.clone();
    every_id.extend(plain_ids);
    let here = kept(&http, &a, &every_id).await;
    let there = kept(&http, &b, &every_id).await;
    println!(
        "kept {} traces of {} in {seconds:.0} s (cap {cap}/s), peer holds {}",
        here.len(),
        every_id.len(),
        there.len()
    );

    assert!(!here.is_empty());
    assert_eq!(
        here, there,
        "the peer must hold exactly what this host kept"
    );
    assert!(
        (here.len() as f64) <= cap * seconds * 1.2 + 50.0,
        "kept {} traces in {seconds} s",
        here.len()
    );
    let kept_debug = debug_ids.iter().filter(|id| here.contains(*id)).count();
    assert!(kept_debug < debug_ids.len() / 2);
}
