use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::Extension;
use axum::Router;
use axum::body::Body;
use axum::http::Request;
use axum::middleware::from_fn_with_state;
use axum::routing::get;
use deadpool_redis::{Config, Runtime};
use futures::future::join_all;
use tokio::net::TcpListener;
use tower::ServiceExt;

use super::*;

#[test]
fn client_identity_groups_ipv6_by_network_and_normalizes_mapped_ipv4() {
    let ipv4 = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7));
    let mapped = IpAddr::V6(Ipv4Addr::new(192, 0, 2, 7).to_ipv6_mapped());
    let first = IpAddr::V6("2001:db8:1234:5678::1".parse::<Ipv6Addr>().unwrap());
    let second = IpAddr::V6("2001:db8:1234:5678:ffff::1".parse::<Ipv6Addr>().unwrap());
    let other = IpAddr::V6("2001:db8:1234:5679::1".parse::<Ipv6Addr>().unwrap());

    assert_eq!(client_identity(ipv4), client_identity(mapped));
    assert_eq!(client_identity(first), client_identity(second));
    assert_ne!(client_identity(first), client_identity(other));
}

#[test]
fn retry_after_rounds_up() {
    assert_eq!(milliseconds_to_seconds(1), 1);
    assert_eq!(milliseconds_to_seconds(1_001), 2);
}

#[test]
fn repeated_failures_share_one_warning_window() -> anyhow::Result<()> {
    let limiter = limiter(
        "redis://127.0.0.1:1",
        limits(1, 10),
        Duration::from_millis(50),
    )?;

    assert!(limiter.warning_due());
    assert!(!limiter.warning_due());
    Ok(())
}

#[tokio::test]
async fn missing_transport_address_fails_before_the_handler() -> anyhow::Result<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let handler_calls = calls.clone();
    let app = Router::new()
        .route(
            "/",
            get(move || {
                let calls = handler_calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::Relaxed);
                    StatusCode::OK
                }
            }),
        )
        .route_layer(from_fn_with_state(
            limiter(
                "redis://127.0.0.1:1",
                limits(1, 10),
                Duration::from_millis(50),
            )?,
            admit_login,
        ));

    let response = app.oneshot(Request::get("/").body(Body::empty())?).await?;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert_eq!(response.headers()[RETRY_AFTER], "1");
    assert_eq!(response.headers()[CACHE_CONTROL], "no-store, private");
    Ok(())
}

#[tokio::test]
async fn stalled_redis_is_bounded_and_fails_closed() -> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let stalled = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.expect("connection should arrive");
        std::future::pending::<()>().await;
    });
    let limiter = limiter(
        &format!("redis://{address}"),
        limits(1, 10),
        Duration::from_millis(50),
    )?;

    let started = Instant::now();
    let decision = limiter
        .check(Endpoint::Login, "127.0.0.1:1234".parse()?)
        .await;
    stalled.abort();

    assert_eq!(decision, Decision::Unavailable);
    assert!(started.elapsed() < Duration::from_millis(500));
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local Redis server"]
async fn per_client_limit_is_shared_across_ports() -> anyhow::Result<()> {
    let limiter = redis_limiter(limits(2, 10)).await?;
    let first: SocketAddr = "192.0.2.7:1000".parse()?;
    let second: SocketAddr = "192.0.2.7:2000".parse()?;
    let other: SocketAddr = "192.0.2.8:1000".parse()?;

    assert_eq!(
        limiter.check(Endpoint::Login, first).await,
        Decision::Allowed
    );
    assert_eq!(
        limiter.check(Endpoint::Login, second).await,
        Decision::Allowed
    );
    assert!(matches!(
        limiter.check(Endpoint::Login, first).await,
        Decision::Limited { .. }
    ));
    assert_eq!(
        limiter.check(Endpoint::Login, other).await,
        Decision::Allowed
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local Redis server"]
async fn concurrent_clients_cannot_exceed_the_global_limit() -> anyhow::Result<()> {
    let limiter = redis_limiter(limits(100, 10)).await?;
    let requests = (1..=40).map(|last_octet| {
        let limiter = limiter.clone();
        async move {
            limiter
                .check(
                    Endpoint::Login,
                    SocketAddr::from(([198, 51, 100, last_octet], 443)),
                )
                .await
        }
    });
    let decisions = join_all(requests).await;

    assert_eq!(
        decisions
            .iter()
            .filter(|decision| **decision == Decision::Allowed)
            .count(),
        10
    );
    assert!(
        decisions
            .iter()
            .all(|decision| *decision != Decision::Unavailable)
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local Redis server"]
async fn forwarded_headers_cannot_change_the_client_bucket() -> anyhow::Result<()> {
    let calls = Arc::new(AtomicUsize::new(0));
    let handler_calls = calls.clone();
    let app = Router::new()
        .route(
            "/",
            get(move || {
                let calls = handler_calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::Relaxed);
                    StatusCode::OK
                }
            }),
        )
        .route_layer(from_fn_with_state(
            redis_limiter(limits(1, 10)).await?,
            admit_login,
        ))
        .layer(Extension(ConnectInfo(SocketAddr::from((
            [203, 0, 113, 7],
            443,
        )))));

    let first = Request::get("/")
        .header("x-forwarded-for", "192.0.2.1")
        .body(Body::empty())?;
    let second = Request::get("/")
        .header("x-forwarded-for", "192.0.2.2")
        .body(Body::empty())?;

    assert_eq!(app.clone().oneshot(first).await?.status(), StatusCode::OK);
    assert_eq!(
        app.oneshot(second).await?.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    Ok(())
}

#[test]
fn each_public_entrance_spends_a_budget_of_its_own() -> anyhow::Result<()> {
    let pool = Config::from_url("redis://127.0.0.1:1").create_pool(Some(Runtime::Tokio1))?;
    let admission = PublicAdmission::with_namespace(
        pool,
        AdmissionCfg {
            window: Duration::from_secs(5),
            timeout: Duration::from_millis(50),
            max_in_flight: 4,
            login: limits(1, 2),
            link_create: limits(3, 4),
            resolve: limits(5, 6),
        },
        "test:public:admission:budgets",
    );

    assert_eq!(admission.limits(Endpoint::Login).per_client, 1);
    assert_eq!(admission.limits(Endpoint::LinkCreate).per_client, 3);
    assert_eq!(
        admission.limits(Endpoint::Resolve).per_client,
        5,
        "a link a listener pasted must not be able to spend the budget that lets them log in"
    );

    let keys = [
        Endpoint::Login.key(),
        Endpoint::LinkCreate.key(),
        Endpoint::Resolve.key(),
    ];
    assert_eq!(
        keys.iter().collect::<std::collections::HashSet<_>>().len(),
        keys.len(),
        "two entrances sharing a counter key share one budget however the config reads: {keys:?}"
    );
    Ok(())
}

#[test]
fn every_open_entrance_that_costs_us_something_outside_is_metered() {
    const METERED: &[(&str, &str, &str)] = &[
        ("src/modules/auth/handlers.rs", "/auth/login", "admit_login"),
        (
            "src/modules/auth/handlers.rs",
            "/auth/link/create",
            "admit_link_create",
        ),
        (
            "src/modules/resolve/handlers.rs",
            "/resolve",
            "admit_resolve",
        ),
    ];

    for (file, path, guard) in METERED {
        let body = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(file),
        )
        .unwrap_or_else(|_| panic!("{file} is readable"));
        let at = body
            .find(&format!("\"{path}\""))
            .unwrap_or_else(|| panic!("{file} no longer declares {path}"));
        let rest = &body[at..];
        let declaration = rest.find(".route(").map_or(rest, |end| &rest[..end]);
        assert!(
            declaration.contains(guard),
            "{path} answers without a session and reaches SoundCloud or writes to the \
             catalogue, so one anonymous caller can spend our relay pool and fill our own \
             tables; it must carry {guard}"
        );
    }
}

fn limits(per_client: u32, global: u32) -> AdmissionLimitCfg {
    AdmissionLimitCfg { per_client, global }
}

fn config(limit: AdmissionLimitCfg, timeout: Duration) -> AdmissionCfg {
    AdmissionCfg {
        window: Duration::from_secs(5),
        timeout,
        max_in_flight: 64,
        login: limit,
        link_create: limit,
        resolve: limit,
    }
}

fn limiter(
    redis_url: &str,
    limit: AdmissionLimitCfg,
    timeout: Duration,
) -> anyhow::Result<Arc<PublicAdmission>> {
    let pool = Config::from_url(redis_url).create_pool(Some(Runtime::Tokio1))?;
    Ok(PublicAdmission::with_namespace(
        pool,
        config(limit, timeout),
        format!("test:public:admission:{}", uuid::Uuid::now_v7()),
    ))
}

async fn redis_limiter(limit: AdmissionLimitCfg) -> anyhow::Result<Arc<PublicAdmission>> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    limiter(&url, limit, Duration::from_secs(2))
}
