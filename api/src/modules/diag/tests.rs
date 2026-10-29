use backend_contracts::diag::{Bundle, EventsBody};
use serde_json::json;
use sqlx::PgPool;

use super::DiagService;

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
    let service = DiagService::new(pool, String::new());

    let (code, _) = service.store_bundle(&bundle()).await?;
    let fetched = service.bundle(&code.to_lowercase()).await?;

    assert_eq!(fetched.expect("found")["app"]["version"], "8.4.25");
    assert!(service.bundle("SCD-00000000").await?.is_none());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn issues_count_installs_apart_from_events(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    let service = DiagService::new(pool, String::new());

    service
        .store_events(&events("0123456789abcdef", "aaaaaaaaaaaa"))
        .await?;
    service
        .store_events(&events("0123456789abcdef", "aaaaaaaaaaaa"))
        .await?;
    service
        .store_events(&events("fedcba9876543210", "bbbbbbbbbbbb"))
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
    assert_eq!(detail.days.len(), 1);
    assert!(service.issue("1111111111111111", 168).await?.is_none());
    Ok(())
}
