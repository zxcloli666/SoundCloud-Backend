use std::time::{Duration, Instant};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::router_search_tests::{app, redis_url};

async fn session_for(pool: &PgPool, user: &str) -> anyhow::Result<Uuid> {
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, $2, $2, 'access', 'refresh', now() + interval '1 hour', '')",
    )
    .bind(connection)
    .bind(user)
    .execute(pool)
    .await?;
    let session = Uuid::now_v7();
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(session)
        .bind(connection)
        .execute(pool)
        .await?;
    Ok(session)
}

async fn call(
    app: &axum::Router,
    session: Uuid,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> anyhow::Result<(StatusCode, Value)> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-session-id", session.to_string());
    if body.is_some() {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
    let response = app.clone().oneshot(request.body(body)?).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

fn playing(urn: &str) -> Value {
    json!({
        "status": "playing",
        "trackUrn": urn,
        "track": {"urn": urn, "title": "Night Drive"},
        "positionMs": 0,
        "leadMs": 800
    })
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_host_and_a_guest_share_one_room(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let host = session_for(&pool, "101").await?;
    let guest = session_for(&pool, "202").await?;

    let (status, room) = call(&app, host, "POST", "/rooms", Some(json!({"name": "Host"}))).await?;
    assert_eq!(status, StatusCode::CREATED, "{room}");
    let code = room["code"].as_str().unwrap_or_default().to_owned();
    assert_eq!(room["hostId"], "101");
    assert_eq!(room["online"], json!(["101"]));

    let (status, _) = call(&app, guest, "GET", &format!("/rooms/{code}"), None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let lower = code.to_lowercase();
    let (status, room) = call(
        &app,
        guest,
        "POST",
        &format!("/rooms/{lower}/members"),
        Some(json!({"name": "Guest", "avatarUrl": "https://i1.sndcdn.com/a.jpg"})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{room}");
    assert_eq!(room["members"].as_array().map(Vec::len), Some(2));
    let joined_version = room["version"].as_u64().unwrap_or_default();

    let (status, _) = call(
        &app,
        guest,
        "PUT",
        &format!("/rooms/{code}/playback"),
        Some(playing("42")),
    )
    .await?;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, room) = call(
        &app,
        host,
        "PUT",
        &format!("/rooms/{code}/playback"),
        Some(playing("42")),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{room}");
    let at = room["playback"]["at"].as_i64().unwrap_or_default();
    let now = room["serverNow"].as_i64().unwrap_or_default();
    assert!(at > now && at <= now + 800, "{room}");

    let started = Instant::now();
    let (status, room) = call(
        &app,
        guest,
        "GET",
        &format!("/rooms/{code}?since={joined_version}"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(room["playback"]["trackUrn"], "soundcloud:tracks:42");
    assert_eq!(room["playback"]["track"]["title"], "Night Drive");

    let (status, room) = call(
        &app,
        guest,
        "PUT",
        &format!("/rooms/{code}/ready"),
        Some(json!({"trackUrn": "42"})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(room["members"][1]["readyUrn"], "soundcloud:tracks:42");

    let (status, _) = call(
        &app,
        guest,
        "DELETE",
        &format!("/rooms/{code}/members/me"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, room) = call(&app, host, "GET", &format!("/rooms/{code}"), None).await?;
    assert_eq!(room["members"].as_array().map(Vec::len), Some(1));

    let (status, _) = call(
        &app,
        host,
        "DELETE",
        &format!("/rooms/{code}/members/me"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&app, host, "GET", &format!("/rooms/{code}"), None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_waiting_guest_wakes_up_on_the_next_change(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let host = session_for(&pool, "303").await?;
    let guest = session_for(&pool, "404").await?;
    let (_, room) = call(&app, host, "POST", "/rooms", Some(json!({"name": "Host"}))).await?;
    let code = room["code"].as_str().unwrap_or_default().to_owned();
    let (_, room) = call(
        &app,
        guest,
        "POST",
        &format!("/rooms/{code}/members"),
        Some(json!({"name": "Guest"})),
    )
    .await?;
    let version = room["version"].as_u64().unwrap_or_default();

    let waiter = {
        let app = app.clone();
        let uri = format!("/rooms/{code}?since={version}");
        tokio::spawn(async move { call(&app, guest, "GET", &uri, None).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    let started = Instant::now();
    call(
        &app,
        host,
        "PUT",
        &format!("/rooms/{code}/playback"),
        Some(playing("soundcloud:tracks:7")),
    )
    .await?;
    let (status, room) = waiter.await??;
    assert_eq!(status, StatusCode::OK);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(room["version"].as_u64(), Some(version + 1));
    assert_eq!(room["playback"]["status"], "playing");
    assert_eq!(room["online"].as_array().map(Vec::len), Some(2));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn malformed_codes_are_unknown_rooms(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, "redis://127.0.0.1:1").await?;
    let guest = session_for(&pool, "505").await?;
    for uri in ["/rooms/ABC", "/rooms/ABC23O", "/rooms/%20"] {
        let (status, _) = call(&app, guest, "GET", uri, None).await?;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
    }
    Ok(())
}
