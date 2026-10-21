use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chrono::Utc;
use sc_chat_wire::bind::make_bind;
use sc_chat_wire::prost::Message;
use sc_chat_wire::proto::{Inner, InnerResponse, Tier};
use sc_chat_wire::random_bytes;
use sc_chat_wire::seal::{RequestSealer, SealedRequest};
use sc_chat_wire::sign::{public_key, sign_request};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::test_helpers::{redis_pool, redis_url, settle, user};
use crate::modules::chat::sync::reply::LATER_MAX_MS;
use crate::modules::chat::sync::wire::RETRY_AFTER_MS;
use crate::modules::chat::test_support::{chat_state, vector_ock};
use crate::modules::chat::{ChatState, routes};

struct Device {
    id: Uuid,
    seed: [u8; 32],
    session: Uuid,
}

fn random() -> [u8; 32] {
    random_bytes().expect("randomness")
}

fn now_ms() -> u64 {
    u64::try_from(Utc::now().timestamp_millis()).expect("a positive clock")
}

async fn enrolled(pool: &PgPool, user_id: &str) -> Device {
    let device = Device {
        id: Uuid::new_v4(),
        seed: random(),
        session: Uuid::now_v7(),
    };
    let box_pub = random();
    let bind = make_bind(&device.id.to_string(), &device.seed, &box_pub, now_ms());
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, $2, $2, 'access', 'refresh', now() + interval '1 hour', '')",
    )
    .bind(connection)
    .bind(user_id)
    .execute(pool)
    .await
    .expect("a connection");
    sqlx::query(
        "INSERT INTO sessions (id, soundcloud_connection_id, chat_bind) VALUES ($1, $2, $3)",
    )
    .bind(device.session)
    .bind(connection)
    .bind(bind.encode_to_vec())
    .execute(pool)
    .await
    .expect("a session");
    sqlx::query(
        "INSERT INTO chat.devices
            (id, user_id, session_id, auth_pub, box_pub, platform, name, app_version, tier)
         VALUES ($1, $2, $3, $4, $5, 'linux', 'test', '0.0.0', 'direct')",
    )
    .bind(device.id)
    .bind(user_id)
    .bind(device.session)
    .bind(public_key(&device.seed).to_vec())
    .bind(box_pub.to_vec())
    .execute(pool)
    .await
    .expect("a device");
    device
}

fn sealed_sync(device: &Device, body: &Value) -> SealedRequest {
    let mut inner = Inner {
        ts_ms: now_ms(),
        session_id: Some(device.session.to_string()),
        device_id: Some(device.id.to_string()),
        method: "POST".to_owned(),
        path: "/chat/sync".to_owned(),
        body: body.to_string().into_bytes(),
        tier: Tier::Direct as i32,
        max_bytes: 262_144,
        ..Inner::default()
    };
    let sealer = RequestSealer::new(1, &vector_ock().public(), random()).expect("a sealer");
    sign_request(&device.seed, &mut inner, sealer.enc());
    sealer
        .seal(&inner.encode_to_vec())
        .expect("a sealed request")
}

async fn exchange(app: Router, request: SealedRequest) -> InnerResponse {
    let response = app
        .oneshot(
            Request::post("/chat/x")
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from(request.envelope.clone()))
                .expect("a request"),
        )
        .await
        .expect("an answer");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("a body");
    let plain = request
        .response_key
        .open(&request.enc, &body)
        .expect("the answer opens");
    InnerResponse::decode(plain.as_slice()).expect("an inner answer")
}

fn header_of<'a>(answer: &'a InnerResponse, name: &str) -> Option<&'a str> {
    answer
        .headers
        .iter()
        .find(|header| header.name == name)
        .map(|header| header.value.as_str())
}

fn opened_to(chat: Arc<ChatState>, user_id: &str) -> Arc<ChatState> {
    let mut chat = chat;
    Arc::get_mut(&mut chat)
        .expect("a fresh state")
        .config
        .beta_users
        .insert(user_id.to_owned());
    chat
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_sealed_poll_parked_at_shutdown_carries_its_retry_delay_to_the_client(pool: PgPool) {
    let bob = user();
    let device = enrolled(&pool, &bob).await;
    let chat = opened_to(
        chat_state(pool.clone(), redis_pool(&redis_url()), true),
        &bob,
    );
    let app: Router = routes::public(chat.clone());

    let first = exchange(
        app.clone(),
        sealed_sync(&device, &json!({"max_bytes": 262_144})),
    )
    .await;
    assert_eq!(first.status, 200);
    let reset: Value = serde_json::from_slice(&first.body).expect("a json answer");
    assert_eq!(reset["reset"], true);

    let parked = tokio::spawn(exchange(
        app,
        sealed_sync(
            &device,
            &json!({
                "pos": reset["pos"],
                "inc": reset["inc"],
                "hold_ms": 20_000,
                "max_bytes": 262_144,
            }),
        ),
    ));
    settle(&chat, 1).await;
    chat.shutdown.cancel();

    let ended = parked.await.expect("a poll");
    assert_eq!(ended.status, 204);
    let retry_ms: u64 = header_of(&ended, RETRY_AFTER_MS)
        .and_then(|value| value.parse().ok())
        .expect("a retry delay");
    assert!(retry_ms <= LATER_MAX_MS);
}
