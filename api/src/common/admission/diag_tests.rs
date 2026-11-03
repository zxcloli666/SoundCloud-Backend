use std::time::Duration;

use deadpool_redis::{Config, Runtime};

use super::*;

fn unlimited() -> AdmissionLimitCfg {
    AdmissionLimitCfg {
        per_client: 1_000_000,
        global: 1_000_000,
    }
}

fn limiter(
    diag_bundle: AdmissionLimitCfg,
    diag_events: AdmissionLimitCfg,
    diag_bytes: AdmissionLimitCfg,
) -> anyhow::Result<Arc<PublicAdmission>> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    let pool = Config::from_url(url).create_pool(Some(Runtime::Tokio1))?;
    let limit = unlimited();
    Ok(PublicAdmission::with_namespace(
        pool,
        AdmissionCfg {
            window: Duration::from_secs(60),
            timeout: Duration::from_secs(2),
            max_in_flight: 64,
            login: limit,
            link_create: limit,
            resolve: limit,
            sc_search: limit,
            catalog_miss: limit,
            room_list: limit,
            room_create: limit,
            room_join: limit,
            diag_bundle,
            diag_events,
            diag_bytes,
            trusted_proxies: Vec::new(),
        },
        format!("test:public:admission:{}", uuid::Uuid::now_v7()),
    ))
}

fn client(name: &str) -> ClientIdentity {
    ClientIdentity(name.to_owned())
}

fn limits(per_client: u32, global: u32) -> AdmissionLimitCfg {
    AdmissionLimitCfg { per_client, global }
}

#[test]
fn the_default_diag_budgets_are_bytes_and_single_events() {
    let cfg = AdmissionCfg::from_env();

    assert_eq!(cfg.diag_bundle.per_client, 10);
    assert_eq!(cfg.diag_events.per_client, 120);
    assert_eq!(cfg.diag_bytes.per_client, 2 * 1024 * 1024);
    assert_eq!(cfg.diag_bytes.global, 200 * 1024 * 1024);
}

#[tokio::test]
#[ignore = "requires a local Redis server"]
async fn bytes_are_spent_per_client_and_per_host_not_per_request() -> anyhow::Result<()> {
    let limiter = limiter(unlimited(), unlimited(), limits(1000, 1500))?;

    assert!(
        limiter
            .admit_diag(Endpoint::DiagBytes, &client("a"), 600)
            .await
            .is_ok()
    );
    assert!(
        limiter
            .admit_diag(Endpoint::DiagBytes, &client("a"), 600)
            .await
            .is_err()
    );
    assert!(
        limiter
            .admit_diag(Endpoint::DiagBytes, &client("b"), 600)
            .await
            .is_ok()
    );
    assert!(
        limiter
            .admit_diag(Endpoint::DiagBytes, &client("c"), 600)
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local Redis server"]
async fn a_post_of_fifty_events_spends_fifty_not_one() -> anyhow::Result<()> {
    let limiter = limiter(unlimited(), limits(120, 100_000), unlimited())?;
    let attacker = client("attacker");

    assert!(
        limiter
            .admit_diag(Endpoint::DiagEvents, &attacker, 50)
            .await
            .is_ok()
    );
    assert!(
        limiter
            .admit_diag(Endpoint::DiagEvents, &attacker, 50)
            .await
            .is_ok()
    );
    assert!(
        limiter
            .admit_diag(Endpoint::DiagEvents, &attacker, 50)
            .await
            .is_err()
    );
    assert!(
        limiter
            .admit_diag(Endpoint::DiagEvents, &attacker, 20)
            .await
            .is_ok()
    );
    assert!(
        limiter
            .admit_diag(Endpoint::DiagEvents, &attacker, 1)
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local Redis server"]
async fn ten_bundles_an_hour_per_client() -> anyhow::Result<()> {
    let limiter = limiter(limits(10, 100_000), unlimited(), unlimited())?;
    let attacker = client("attacker");

    for _ in 0..10 {
        assert!(
            limiter
                .admit_diag(Endpoint::DiagBundle, &attacker, 1)
                .await
                .is_ok()
        );
    }
    let refused = limiter
        .admit_diag(Endpoint::DiagBundle, &attacker, 1)
        .await
        .expect_err("the eleventh bundle is refused");

    assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        limiter
            .admit_diag(Endpoint::DiagBundle, &client("other"), 1)
            .await
            .is_ok()
    );
    Ok(())
}
