use backend_contracts::diag::{Bundle, EventsBody};
use backend_contracts::diag_report::{Rule, View};
use serde_json::json;
use sqlx::PgPool;

use super::DiagService;
use crate::common::admission::ClientIdentity;
use crate::config::DiagCfg;

pub(super) fn config() -> DiagCfg {
    DiagCfg {
        host: "main".to_owned(),
        code_slot: 0,
        max_bytes: 1 << 30,
        origin_key: "test-origin-key".to_owned().into(),
        jaeger_url: String::new(),
        sourcemaps_dir: std::env::temp_dir()
            .join("scd-test-sourcemaps")
            .to_string_lossy()
            .into_owned(),
    }
}

pub(super) fn client(address: &str) -> ClientIdentity {
    ClientIdentity(address.to_owned())
}

async fn install(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!("../../../migrations/0156_diag.sql"))
        .execute(pool)
        .await?;
    Ok(())
}

fn app() -> serde_json::Value {
    json!({
        "version": "8.4.25", "os": "windows", "os_version": "10", "arch": "x86_64",
        "install": "nsis", "locale": "ru", "id": "a1b2c3d4e5f6"
    })
}

fn bundle() -> Bundle {
    serde_json::from_value(json!({
        "kind": "scd-diagnostics", "v": 1, "at": "2026-10-29T10:00:00.000Z", "app": app(),
        "session": { "uptime_s": 1, "logged_in": true, "premium": false, "quality": "hq", "route": {} },
        "trigger": { "kind": "manual", "error": null },
        "attempts": [], "errors": [], "netcheck": null, "log_tail": []
    }))
    .expect("a bundle")
}

fn events(fingerprint: &str, id: &str) -> EventsBody {
    let mut app = app();
    app["id"] = json!(id);
    serde_json::from_value(json!({
        "app": app,
        "events": [{
            "at": "2026-10-29T10:00:00.000Z", "kind": "render", "message": "boom\nsecond line",
            "stack": null, "component_stack": null, "trace": null,
            "fingerprint": fingerprint, "count": 1, "context": {}
        }]
    }))
    .expect("events")
}

#[sqlx::test(migrations = false)]
async fn a_stored_bundle_comes_back_by_any_spelling_of_its_code(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    let service = DiagService::new(pool, &config());

    let (code, _) = service.store_bundle(&bundle()).await?;
    let fetched = service.bundle(&code.to_lowercase()).await?;

    assert_eq!(fetched.expect("found")["app"]["version"], "8.4.25");
    assert!(service.bundle("SCD-00000000").await?.is_none());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn issues_count_installs_apart_from_events(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    let service = DiagService::new(pool, &config());

    service
        .store_events(&events("0123456789abcdef", "aaaaaaaaaaaa"), &client("one"))
        .await?;
    service
        .store_events(&events("0123456789abcdef", "aaaaaaaaaaaa"), &client("one"))
        .await?;
    service
        .store_events(&events("fedcba9876543210", "bbbbbbbbbbbb"), &client("two"))
        .await?;

    let issues = service.issues(24, 50).await?;
    let first = &issues[0];
    assert_eq!(first.fingerprint, "0123456789abcdef");
    assert_eq!((first.events, first.installs), (2, 1));
    assert_eq!(first.title, "boom");
    assert_eq!(first.versions["8.4.25"], 2);
    assert_eq!(issues.len(), 2);
    let detail = service
        .issue("0123456789abcdef", 168)
        .await?
        .expect("found");
    assert_eq!(detail.events.len(), 2);
    assert_eq!(detail.days.last().map(|day| day.installs), Some(1));
    assert!(service.issue("1111111111111111", 168).await?.is_none());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_report_counts_traces_that_could_not_be_read(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    let service = DiagService::new(pool, &config());
    let mut with_attempt = bundle();
    with_attempt.attempts = serde_json::from_value(json!([{
        "at": "2026-10-29T10:00:00.000Z", "trace": "4bf92f3577b3da6a3ce929d0e0e4736f",
        "span": "00f067aa0ba902b7", "debug": true, "service": "stream", "method": "GET",
        "path": "/stream/soundcloud:tracks:1", "hop": "r2", "status": 0, "ms": 10012,
        "error": "connect timeout after 10s", "bytes": 0
    }]))?;
    let (code, _) = service.store_bundle(&with_attempt).await?;

    let internal = service
        .report(&code, View::Internal)
        .await?
        .expect("report");
    let public = service.report(&code, View::Public).await?.expect("report");

    assert_eq!(internal.counts.traces_wanted, 1);
    assert_eq!(internal.missing.trace_ids.len(), 1);
    assert!(public.missing.trace_ids.is_empty());
    assert!(public.header.app.id.is_none());
    assert!(
        internal
            .hints
            .iter()
            .any(|hint| hint.rule == Rule::NeverReached)
    );
    assert!(
        service
            .report("SCD-00000000", View::Internal)
            .await?
            .is_none()
    );
    Ok(())
}
