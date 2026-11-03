use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

use super::router_search_tests::{redis_url, soundcloud_hits, state};
use crate::modules::diag::DiagService;

const TOKEN: &str = "regression-diag-token";

fn bundle() -> String {
    json!({
        "kind": "scd-diagnostics", "v": 1, "at": "2026-10-29T10:00:00.000Z",
        "app": { "version": "8.4.25", "os": "windows", "os_version": "10", "arch": "x86_64",
                 "install": "nsis", "locale": "ru", "id": "a1b2c3d4e5f6" },
        "session": { "uptime_s": 1, "logged_in": false, "premium": false, "quality": "hq", "route": {} },
        "trigger": { "kind": "manual", "error": null },
        "attempts": [], "errors": [], "netcheck": null, "log_tail": []
    })
    .to_string()
}

fn events(fingerprint: &str, message: &str) -> String {
    json!({
        "app": { "version": "8.4.25", "os": "windows", "os_version": "10", "arch": "x86_64",
                 "install": "nsis", "locale": "ru", "id": "a1b2c3d4e5f6" },
        "events": [{
            "at": "2026-10-29T10:00:00.000Z", "kind": "render", "message": message,
            "stack": null, "component_stack": null, "trace": null,
            "fingerprint": fingerprint, "count": 1, "context": {}
        }]
    })
    .to_string()
}

fn from_a_fresh_peer(mut request: Request<Body>) -> Request<Body> {
    let octets = uuid::Uuid::now_v7().into_bytes();
    let peer = SocketAddr::from(([octets[10], octets[11], octets[12], octets[13]], 40000));
    request.extensions_mut().insert(ConnectInfo(peer));
    request
}

fn post(path: &str, content_type: &str, body: impl Into<Body>) -> Request<Body> {
    from_a_fresh_peer(
        Request::post(path)
            .header("content-type", content_type)
            .body(body.into())
            .expect("request"),
    )
}

async fn app(pool: &PgPool, token: &str, max_bytes: u64) -> anyhow::Result<axum::Router> {
    let mut built = state(pool, soundcloud_hits(), &redis_url()).await?;
    let mut config = (*built.config).clone();
    config.admin.diag_token = token.to_owned().into();
    config.diag.max_bytes = max_bytes;
    built.diag = DiagService::new(pool.clone(), &config.diag);
    built.config = Arc::new(config);
    Ok(crate::router::build(built))
}

async fn body_of(response: axum::response::Response) -> anyhow::Result<Value> {
    let bytes = to_bytes(response.into_body(), 1 << 20).await?;
    Ok(serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn admin_get(path: &str) -> anyhow::Result<Request<Body>> {
    Ok(Request::get(path)
        .header("x-diag-token", TOKEN)
        .body(Body::empty())?)
}

#[sqlx::test(migrations = "./migrations")]
async fn a_web_page_cannot_post_diagnostics_or_read_the_code(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, TOKEN, 1 << 30).await?;

    let simple = post("/diag/bundles", "text/plain;charset=UTF-8", bundle());
    assert_eq!(
        app.clone().oneshot(simple).await?.status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );

    let mut from_a_page = post("/diag/bundles", "application/json", bundle());
    from_a_page
        .headers_mut()
        .insert("origin", "https://any-site.example".parse()?);
    let response = app.clone().oneshot(from_a_page).await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let mut events_from_a_page = post(
        "/diag/events",
        "application/json",
        events("0123456789abcdef", "x"),
    );
    events_from_a_page
        .headers_mut()
        .insert("origin", "https://any-site.example".parse()?);
    assert_eq!(
        app.clone().oneshot(events_from_a_page).await?.status(),
        StatusCode::FORBIDDEN
    );
    let text_events = post(
        "/diag/events",
        "text/plain",
        events("0123456789abcdef", "x"),
    );
    assert_eq!(
        app.oneshot(text_events).await?.status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn desktop_style_posts_work_and_answers_are_never_cached(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, TOKEN, 1 << 30).await?;

    let created = app
        .clone()
        .oneshot(post("/diag/bundles", "application/json", bundle()))
        .await?;
    assert_eq!(created.status(), StatusCode::CREATED);
    let code = body_of(created).await?["code"]
        .as_str()
        .expect("code")
        .to_owned();
    let accepted = app
        .clone()
        .oneshot(post(
            "/diag/events",
            "application/json; charset=utf-8",
            events("0123456789abcdef", "boom"),
        ))
        .await?;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);

    let oversized = post(
        "/diag/bundles",
        "application/json",
        vec![b' '; 256 * 1024 + 1],
    );
    assert_eq!(
        app.clone().oneshot(oversized).await?.status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );

    let read = app
        .clone()
        .oneshot(admin_get(&format!("/admin/diag/bundles/{code}"))?)
        .await?;
    assert_eq!(read.status(), StatusCode::OK);
    let headers = read.headers();
    assert_eq!(
        headers.get("cache-control").and_then(|v| v.to_str().ok()),
        Some("private, no-store, max-age=0")
    );
    assert!(
        headers
            .get("vary")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|vary| vary.contains("x-diag-token"))
    );

    let refused = app
        .clone()
        .oneshot(Request::get("/admin/diag/issues").body(Body::empty())?)
        .await?;
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        refused
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("private, no-store, max-age=0")
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_diag_token_opens_diag_routes_only_and_an_empty_one_opens_nothing(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app(&pool, TOKEN, 1 << 30).await?;
    let wrong_scope = Request::get("/admin/auth/overview")
        .header("x-diag-token", TOKEN)
        .body(Body::empty())?;
    assert_eq!(
        app.oneshot(wrong_scope).await?.status(),
        StatusCode::UNAUTHORIZED
    );

    let unset = self::app(&pool, "", 1 << 30).await?;
    for header in ["", " "] {
        let empty = Request::get("/admin/diag/bundles/SCD-00000000")
            .header("x-diag-token", header)
            .body(Body::empty())?;
        assert_eq!(
            unset.clone().oneshot(empty).await?.status(),
            StatusCode::UNAUTHORIZED
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn both_posts_answer_503_while_the_tables_are_over_the_ceiling(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app(&pool, TOKEN, 1).await?;

    let bundle = app
        .clone()
        .oneshot(post("/diag/bundles", "application/json", bundle()))
        .await?;
    let events = app
        .oneshot(post(
            "/diag/events",
            "application/json",
            events("0123456789abcdef", "boom"),
        ))
        .await?;

    assert_eq!(bundle.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(events.status(), StatusCode::SERVICE_UNAVAILABLE);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_server_computes_the_fingerprint_and_ignores_the_clients(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app(&pool, TOKEN, 1 << 30).await?;
    let forged = "ffffffffffffffff";

    let accepted = app
        .clone()
        .oneshot(post(
            "/diag/events",
            "application/json",
            events(forged, "Cannot read properties of undefined"),
        ))
        .await?;
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);

    let issues = body_of(
        app.oneshot(admin_get("/admin/diag/issues?hours=24")?)
            .await?,
    )
    .await?;
    let stored = issues[0]["fingerprint"].as_str().expect("fingerprint");
    assert_ne!(stored, forged);
    assert_eq!(stored.len(), 16);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_premium_reserve_node_takes_reports_and_diag_reads_without_a_session(
    pool: PgPool,
) -> anyhow::Result<()> {
    let mut built = state(&pool, soundcloud_hits(), &redis_url()).await?;
    let mut config = (*built.config).clone();
    config.admin.diag_token = TOKEN.to_owned().into();
    config.premium_reserve = true;
    built.diag = DiagService::new(pool.clone(), &config.diag);
    built.config = Arc::new(config);
    let app = crate::router::build(built);

    let posted = app
        .clone()
        .oneshot(post("/diag/bundles", "application/json", bundle()))
        .await?;
    assert_eq!(posted.status(), StatusCode::CREATED);

    let read = app
        .clone()
        .oneshot(admin_get("/admin/diag/issues")?)
        .await?;
    assert_eq!(read.status(), StatusCode::OK);

    let wrong_scope = Request::get("/admin/auth/overview")
        .header("x-diag-token", TOKEN)
        .body(Body::empty())?;
    assert_ne!(app.oneshot(wrong_scope).await?.status(), StatusCode::OK);
    Ok(())
}
