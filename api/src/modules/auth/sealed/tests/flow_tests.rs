use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::support::{
    Device, PEER, app, call, cfg, exchange, from, link_create, login, oauth_app, seal, sealed,
    send, session_for,
};

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_sealed_login_starts_and_polls_through_auth_x(pool: PgPool) -> anyhow::Result<()> {
    oauth_app(&pool).await?;
    let node = sealed(cfg(), &pool)?;
    let router = app(&node);
    let device = Device::new();

    let (status, started) = send(&router, login(&device), &device).await;

    assert_eq!(status, 200, "{started}");
    assert!(
        started["url"]
            .as_str()
            .is_some_and(|url| url.contains("state="))
    );
    let id = started["loginRequestId"].as_str().expect("a request id");
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM login_requests WHERE id = $1::uuid")
        .bind(id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(rows, 1);

    let poll = call("GET", "/auth/login/status", &format!("id={id}"), &device);
    let (status, polled) = send(&router, poll, &device).await;
    assert_eq!(status, 200, "{polled}");
    assert_eq!(polled["status"], "pending");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_sealed_link_runs_create_claim_and_status(pool: PgPool) -> anyhow::Result<()> {
    let node = sealed(cfg(), &pool)?;
    let router = app(&node);
    let source = Device::new();
    let source_session = session_for(&pool, "7301", Some(&source)).await?;
    let fresh = Device::new();

    let (status, created) = send(&router, link_create(&fresh, "pull"), &fresh).await;
    assert_eq!(status, 200, "{created}");
    let request_id = created["linkRequestId"].as_str().expect("a link id");
    let token = created["claimToken"].as_str().expect("a claim token");

    let mut claim = call("POST", "/auth/link/claim", "", &source);
    claim.session_id = Some(source_session.to_string());
    claim.body = json!({ "claimToken": token }).to_string().into_bytes();
    let (status, claimed) = send(&router, claim, &source).await;
    assert_eq!(status, 200, "{claimed}");
    let new_session = claimed["sessionId"].as_str().expect("a session");

    let poll = call(
        "GET",
        "/auth/link/status",
        &format!("id={request_id}"),
        &fresh,
    );
    let (status, polled) = send(&router, poll, &fresh).await;
    assert_eq!(status, 200, "{polled}");
    assert_eq!(polled["status"], "claimed");
    assert_eq!(polled["sessionId"], new_session);

    let stranger = Device::new();
    let poll = call(
        "GET",
        "/auth/link/status",
        &format!("id={request_id}"),
        &stranger,
    );
    let (_, polled) = send(&router, poll, &stranger).await;
    assert!(polled.get("sessionId").is_none(), "{polled}");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_replayed_envelope_is_refused(pool: PgPool) -> anyhow::Result<()> {
    oauth_app(&pool).await?;
    let node = sealed(cfg(), &pool)?;
    let router = app(&node);
    let device = Device::new();
    let request = seal(login(&device), &device);

    let (first, _) = exchange(&router, &request, from(PEER)).await;
    let (second, body) = exchange(&router, &request, from(PEER)).await;

    assert_eq!(first, 200);
    assert_eq!((second, body["error"].as_str()), (409, Some("chat_replay")));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn any_other_path_is_not_found(pool: PgPool) -> anyhow::Result<()> {
    let node = sealed(cfg(), &pool)?;
    let router = app(&node);
    let device = Device::new();
    for path in ["/chat/config", "/auth/session", "/auth/logout", "/auth/x"] {
        let (status, body) = send(&router, call("GET", path, "", &device), &device).await;
        assert_eq!(
            (status, body["error"].as_str()),
            (404, Some("chat_not_found")),
            "{path}"
        );
    }
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_request_without_a_transport_address_is_unavailable(pool: PgPool) -> anyhow::Result<()> {
    let node = sealed(cfg(), &pool)?;
    let router = app(&node);
    let device = Device::new();
    let (status, body) = exchange(&router, &seal(login(&device), &device), None).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (403, Some("chat_unavailable"))
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_unsigned_or_unbound_request_is_invalid(pool: PgPool) -> anyhow::Result<()> {
    let node = sealed(cfg(), &pool)?;
    let router = app(&node);
    let device = Device::new();
    let stranger = Device::new();

    let mut unbound = login(&device);
    unbound.bind = None;
    let (status, body) = send(&router, unbound, &device).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("chat_invalid"))
    );

    let (status, body) = send(&router, login(&device), &stranger).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (400, Some("chat_invalid"))
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_plain_switch_turns_starts_off_and_leaves_polls_on(pool: PgPool) -> anyhow::Result<()> {
    let mut config = cfg();
    config.plain = true;
    let node = sealed(config, &pool)?;
    let router = app(&node);
    let device = Device::new();

    for inner in [login(&device), link_create(&device, "pull")] {
        let (status, body) = send(&router, inner, &device).await;
        assert_eq!(
            (status, body["error"].as_str()),
            (503, Some("chat_disabled"))
        );
    }
    let poll = call(
        "GET",
        "/auth/login/status",
        &format!("id={}", Uuid::new_v4()),
        &device,
    );
    let (status, body) = send(&router, poll, &device).await;
    assert_eq!(status, 200, "{body}");
    Ok(())
}

#[tokio::test]
async fn the_route_is_absent_while_auth_x_is_off() -> anyhow::Result<()> {
    let router = crate::modules::auth::sealed::router::<()>(None);
    let response = tower::ServiceExt::oneshot(
        router,
        axum::http::Request::post("/auth/x")
            .body(axum::body::Body::empty())
            .expect("a request"),
    )
    .await?;
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    Ok(())
}
