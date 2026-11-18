use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::router_search_tests::{app, config, redis_url};
use crate::config::{AdmissionLimitCfg, AppConfig};

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

struct Reply {
    status: StatusCode,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    headers: &[(&str, String)],
    body: Option<(&str, String)>,
) -> anyhow::Result<Reply> {
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let body = match body {
        Some((content_type, text)) => {
            request = request.header(header::CONTENT_TYPE, content_type);
            Body::from(text)
        }
        None => Body::empty(),
    };
    answer_to(app, request.body(body)?).await
}

async fn answer_to(app: &axum::Router, request: Request<Body>) -> anyhow::Result<Reply> {
    let response = app.clone().oneshot(request).await?;
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await?.to_vec();
    Ok(Reply { status, body })
}

fn with_session(session: Uuid) -> Vec<(&'static str, String)> {
    vec![("x-session-id", session.to_string())]
}

async fn json_call(
    app: &axum::Router,
    session: Uuid,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> anyhow::Result<Reply> {
    send(
        app,
        method,
        uri,
        &with_session(session),
        body.map(|body| ("application/json", body.to_string())),
    )
    .await
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn resolve_merges_variants_and_matches_case_forms(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source, track_count_primary, popularity_score)
         VALUES ('00000000-0000-0000-0000-0000000000a1', 'Земфира', 'земфира', 'test', 40, 0.9),
                ('00000000-0000-0000-0000-0000000000a2', 'Земфирка Мэш', 'земфирка мэш', 'test', 3, 0.1)",
    )
    .execute(&pool)
    .await?;
    crate::db::insert_replicated(
        &pool,
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, metadata_artist,
                             uploader_username, uploader_sc_user_id, duration_ms, play_count_sc, sharing)
         VALUES ('9001', 'soundcloud:tracks:9001', 'Хочешь', 'хочешь', 'Земфира', 'Земфира', '555', 200000, 50000, 'public'),
                ('9002', 'soundcloud:tracks:9002', 'Хочешь (remix)', 'хочешь remix', 'Земфира', 'Земфира', '555', 210000, 10, 'public')",
    )
    .await?;
    crate::db::replicate_track_search(&pool).await?;
    crate::modules::search::lexicon_refresh::refresh(&pool).await?;
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;

    let found = json_call(
        &app,
        session,
        "POST",
        "/search/resolve",
        Some(json!({
            "variants": ["песни земфиры", "земфиры", "земфира хочешь", "земфиры"],
            "kinds": ["artist", "track"],
            "limit": 3
        })),
    )
    .await?;
    assert_eq!(found.status, StatusCode::OK, "{}", found.text());
    let body = found.json();
    let artists = body["results"]["artist"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(artists[0]["name"], "Земфира", "{body}");
    assert_eq!(artists[0]["kind"], "artist");
    assert!(artists[0]["score"].as_f64().unwrap_or_default() > 0.3);
    assert!(artists[0]["popularity"].as_f64().unwrap_or_default() > 0.5);
    let ids: Vec<&str> = artists.iter().filter_map(|a| a["id"].as_str()).collect();
    let unique: std::collections::HashSet<&&str> = ids.iter().collect();
    assert_eq!(
        ids.len(),
        unique.len(),
        "variants merge into one row per entity"
    );

    let tracks = body["results"]["track"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(tracks[0]["id"], "soundcloud:tracks:9001", "{body}");
    assert_eq!(tracks[0]["artist"], "Земфира");
    assert!(body["results"].get("album").is_none());

    let bad = json_call(
        &app,
        session,
        "POST",
        "/search/resolve",
        Some(json!({"variants": ["x"], "kinds": ["podcast"]})),
    )
    .await?;
    assert_eq!(bad.status, StatusCode::UNPROCESSABLE_ENTITY);

    let empty = json_call(
        &app,
        session,
        "POST",
        "/search/resolve",
        Some(json!({"variants": []})),
    )
    .await?;
    assert_eq!(empty.status, StatusCode::OK);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis and NATS"]
async fn resolve_with_a_hostile_body_stays_fast(pool: PgPool) -> anyhow::Result<()> {
    let app = app(&pool, &redis_url()).await?;
    let session = session_for(&pool, "101").await?;
    let started = std::time::Instant::now();
    let reply = json_call(
        &app,
        session,
        "POST",
        "/search/resolve",
        Some(json!({
            "variants": (0..150).map(|n| format!("{}{n}", "я".repeat(300))).collect::<Vec<_>>(),
            "kinds": vec!["track"; 5_000],
        })),
    )
    .await?;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    assert!(started.elapsed() < Duration::from_secs(3));
    Ok(())
}

fn tuned(tweak: impl FnOnce(&mut AppConfig)) -> AppConfig {
    let mut config = config(&redis_url());
    tweak(&mut config);
    config
}

fn cap(per_client: u32) -> AdmissionLimitCfg {
    AdmissionLimitCfg {
        per_client,
        global: 1_000_000,
    }
}

#[path = "router_voice_chat_tests.rs"]
mod chat;
#[path = "router_voice_speak_tests.rs"]
mod speak;
