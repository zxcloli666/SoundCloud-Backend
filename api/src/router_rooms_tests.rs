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

fn hosts(list: &Value) -> Vec<String> {
    list["collection"]
        .as_array()
        .map(|rooms| {
            rooms
                .iter()
                .filter_map(|room| room["hostId"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_public_room_is_listed_and_joined_without_its_code(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let host_id = format!("6{}", Uuid::now_v7().as_u128() % 1_000_000_000);
    let host = session_for(&pool, &host_id).await?;
    let guest = session_for(&pool, "707").await?;
    let join = format!("/rooms/public/{host_id}/members");

    let (_, room) = call(&app, host, "POST", "/rooms", Some(json!({"name": "Host"}))).await?;
    let code = room["code"].as_str().unwrap_or_default().to_owned();
    assert_eq!(room["public"], false);
    let (status, list) = call(&app, guest, "GET", "/rooms/public", None).await?;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert!(!hosts(&list).contains(&host_id));
    let (status, _) = call(&app, guest, "POST", &join, Some(json!({"name": "Guest"}))).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let visibility = format!("/rooms/{code}/visibility");
    let open = Some(json!({"public": true}));
    let (status, _) = call(&app, guest, "PUT", &visibility, open.clone()).await?;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, room) = call(&app, host, "PUT", &visibility, open).await?;
    assert_eq!(status, StatusCode::OK, "{room}");
    assert_eq!(room["public"], true);
    assert_eq!(room["code"], code.as_str());
    call(
        &app,
        host,
        "PUT",
        &format!("/rooms/{code}/playback"),
        Some(playing("42")),
    )
    .await?;

    let (_, list) = call(&app, host, "GET", "/rooms/public", None).await?;
    assert!(
        !hosts(&list).contains(&host_id),
        "a host does not see their own room"
    );
    let (_, list) = call(&app, guest, "GET", "/rooms/public", None).await?;
    let card = list["collection"]
        .as_array()
        .and_then(|rooms| rooms.iter().find(|room| room["hostId"] == host_id.as_str()))
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        card["hostName"],
        host_id.as_str(),
        "the name comes from the account, not from the request"
    );
    assert_eq!(card["listeners"], 1);
    assert_eq!(card["capacity"], 10);
    assert_eq!(card["full"], false);
    assert!(!list.to_string().contains(&code), "{list}");

    let (status, room) = call(&app, guest, "POST", &join, Some(json!({"name": "Guest"}))).await?;
    assert_eq!(status, StatusCode::OK, "{room}");
    assert_eq!(room["code"], code.as_str());
    assert_eq!(room["members"].as_array().map(Vec::len), Some(2));

    sqlx::query(
        "INSERT INTO user_blocked_artists (sc_user_id, kind, target_id, name, sc_user_ids)
         VALUES ('808', 'user', $1, 'Host', ARRAY[$1])",
    )
    .bind(&host_id)
    .execute(&pool)
    .await?;
    let blocker = session_for(&pool, "808").await?;
    let (_, list) = call(&app, blocker, "GET", "/rooms/public", None).await?;
    assert!(!hosts(&list).contains(&host_id));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_room_born_public_changes_its_code_when_it_goes_private(
    pool: PgPool,
) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let host_id = format!("7{}", Uuid::now_v7().as_u128() % 1_000_000_000);
    let host = session_for(&pool, &host_id).await?;
    let guest = session_for(&pool, "717").await?;
    let stranger = session_for(&pool, "919").await?;
    let join = format!("/rooms/public/{host_id}/members");

    let born = Some(json!({"name": "Host", "public": true}));
    let (status, room) = call(&app, host, "POST", "/rooms", born).await?;
    assert_eq!(status, StatusCode::CREATED, "{room}");
    assert_eq!(room["public"], true);
    let code = room["code"].as_str().unwrap_or_default().to_owned();
    let (_, list) = call(&app, guest, "GET", "/rooms/public", None).await?;
    assert!(
        hosts(&list).contains(&host_id),
        "listed from the first second"
    );
    let (status, _) = call(&app, guest, "POST", &join, Some(json!({"name": "Guest"}))).await?;
    assert_eq!(status, StatusCode::OK);

    let (status, room) = call(
        &app,
        host,
        "PUT",
        &format!("/rooms/{code}/visibility"),
        Some(json!({"public": false})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{room}");
    assert_eq!(room["public"], false);
    let moved = room["code"].as_str().unwrap_or_default().to_owned();
    assert_ne!(moved, code);
    assert_eq!(room["members"].as_array().map(Vec::len), Some(2));
    assert_eq!(room["online"].as_array().map(Vec::len), Some(2));

    let (_, list) = call(&app, stranger, "GET", "/rooms/public", None).await?;
    assert!(!hosts(&list).contains(&host_id));
    let late = Some(json!({"name": "Late"}));
    let (status, _) = call(&app, stranger, "POST", &join, late.clone()).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let by_old_code = format!("/rooms/{code}/members");
    let (status, _) = call(&app, stranger, "POST", &by_old_code, late).await?;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the listed room's code is dead"
    );

    let (status, _) = call(&app, guest, "GET", &format!("/rooms/{code}"), None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let follow = format!("/rooms/{code}?since=1&follow=true");
    let (status, _) = call(&app, stranger, "GET", &follow, None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "only members are led over");
    let (status, room) = call(&app, guest, "GET", &follow, None).await?;
    assert_eq!(status, StatusCode::OK, "{room}");
    assert_eq!(room["code"], moved.as_str());

    let (status, refusal) = call(&app, stranger, "GET", &format!("/rooms/{moved}"), None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(refusal["code"], "room_not_member");

    let leave = format!("/rooms/{code}/members/me");
    let (status, _) = call(&app, guest, "DELETE", &leave, None).await?;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, room) = call(&app, host, "GET", &format!("/rooms/{moved}"), None).await?;
    assert_eq!(room["members"].as_array().map(Vec::len), Some(1));

    let (status, next) = call(&app, host, "POST", "/rooms", Some(json!({"name": "Host"}))).await?;
    assert_eq!(status, StatusCode::CREATED);
    let next_code = next["code"].as_str().unwrap_or_default().to_owned();
    assert_ne!(next_code, moved);
    let (status, _) = call(&app, host, "GET", &format!("/rooms/{moved}"), None).await?;
    assert_eq!(status, StatusCode::NOT_FOUND, "a host keeps one room");
    call(
        &app,
        host,
        "DELETE",
        &format!("/rooms/{next_code}/members/me"),
        None,
    )
    .await?;
    Ok(())
}
