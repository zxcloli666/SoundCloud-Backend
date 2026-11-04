use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::modules::chat::media::view;
use crate::modules::chat::routes::inner;
use crate::modules::chat::test_support::{caller, chat_state, offline_chat_state};
use crate::modules::chat::{ChatCaller, ChatState};
use crate::modules::subscriptions::SubscriptionsService;

const FAR: i64 = 4_000_000_000;

fn redis() -> deadpool_redis::Pool {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    crate::redis::connect_url(&url).expect("a redis pool")
}

fn fresh_caller() -> ChatCaller {
    ChatCaller {
        user_id: format!("{}", 2_000_000_000 + u64::from(rand::random::<u32>())),
        ..caller()
    }
}

fn uploads_on(mut chat: Arc<ChatState>) -> Arc<ChatState> {
    Arc::get_mut(&mut chat)
        .expect("a fresh state")
        .config
        .media
        .uploads = true;
    chat
}

async fn call(
    chat: Arc<ChatState>,
    who: &ChatCaller,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let payload = match body {
        Some(body) => {
            builder = builder.header("content-type", "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let mut request = builder.body(payload).expect("a request");
    request.extensions_mut().insert(who.clone());
    let response = inner()
        .with_state(chat)
        .oneshot(request)
        .await
        .expect("an answer");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("a body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn reserve_body(kind: &str, bytes: u64, thumb: Option<u64>) -> Value {
    let mut body = json!({
        "upload_id": Uuid::new_v4(), "conversation_id": Uuid::new_v4(),
        "kind": kind, "bytes": bytes
    });
    if let Some(thumb) = thumb {
        body["thumb_bytes"] = json!(thumb);
    }
    body
}

fn code(answer: &(StatusCode, Value)) -> (u16, String) {
    (
        answer.0.as_u16(),
        answer.1["error"].as_str().unwrap_or_default().to_owned(),
    )
}

#[tokio::test]
async fn with_uploads_off_reserve_is_paused_before_anything_else() {
    let chat = offline_chat_state(true);
    let who = fresh_caller();
    for body in [
        reserve_body("image", 151_552, Some(24_576)),
        reserve_body("image", 1, None),
        json!({"nonsense": true}),
    ] {
        let answer = call(
            chat.clone(),
            &who,
            "POST",
            "/chat/media/reserve",
            Some(body),
        )
        .await;
        assert_eq!(
            code(&answer),
            (503, "chat_media_uploads_paused".to_owned()),
            "{answer:?}"
        );
    }
}

#[tokio::test]
async fn a_bad_shape_is_invalid_before_the_star_check() {
    let chat = uploads_on(offline_chat_state(true));
    let who = fresh_caller();
    for body in [
        reserve_body("image", 151_552, None),
        reserve_body("avatar", 16_384, Some(4_096)),
        reserve_body("image", 100, Some(4_096)),
        reserve_body("thumb", 4_096, None),
        reserve_body("avatar", 1 << 20, None),
    ] {
        let answer = call(
            chat.clone(),
            &who,
            "POST",
            "/chat/media/reserve",
            Some(body.clone()),
        )
        .await;
        assert_eq!(code(&answer), (400, "chat_invalid".to_owned()), "{body}");
    }
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn reserve_asks_for_star_before_the_membership(pool: PgPool) {
    let chat = uploads_on(chat_state(pool.clone(), redis(), true));
    let free = fresh_caller();
    let star = fresh_caller();
    SubscriptionsService::new(pool, false)
        .upsert(&star.user_id, FAR, Some(1))
        .await
        .expect("a subscription");
    let body = reserve_body("image", 151_552, Some(24_576));
    let refused = call(
        chat.clone(),
        &free,
        "POST",
        "/chat/media/reserve",
        Some(body.clone()),
    )
    .await;
    assert_eq!(code(&refused), (403, "chat_media_star_required".to_owned()));
    let admitted = call(chat, &star, "POST", "/chat/media/reserve", Some(body)).await;
    assert_eq!(code(&admitted), (403, "chat_not_member".to_owned()));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_read_and_write_routes_check_their_input(pool: PgPool) {
    let chat = chat_state(pool, redis(), true);
    let who = fresh_caller();
    let id = Uuid::new_v4();
    let ids = |count: usize| -> Vec<Uuid> { (0..count).map(|_| Uuid::new_v4()).collect() };
    let cases: Vec<(&str, &str, Option<Value>, u16, &str)> = vec![
        (
            "POST",
            "/chat/media/urls",
            Some(json!({"ids": ids(1)})),
            200,
            "",
        ),
        (
            "POST",
            "/chat/media/urls",
            Some(json!({"ids": ids(100)})),
            200,
            "",
        ),
        (
            "POST",
            "/chat/media/urls",
            Some(json!({"ids": ids(101)})),
            400,
            "chat_invalid",
        ),
        (
            "POST",
            "/chat/media/urls",
            Some(json!({"ids": []})),
            400,
            "chat_invalid",
        ),
        (
            "POST",
            "/chat/media/urls",
            Some(json!({"ids": [id, id]})),
            400,
            "chat_invalid",
        ),
        (
            "POST",
            "/chat/media/urls",
            Some(json!({"nonsense": 1})),
            400,
            "chat_invalid",
        ),
        ("GET", "/chat/media/usage", None, 200, ""),
        ("GET", "/chat/media/mine", None, 200, ""),
        ("GET", "/chat/media/mine?order=big&limit=200", None, 200, ""),
        (
            "GET",
            "/chat/media/mine?limit=201",
            None,
            400,
            "chat_invalid",
        ),
        ("GET", "/chat/media/mine?limit=0", None, 400, "chat_invalid"),
        (
            "GET",
            "/chat/media/mine?order=sideways",
            None,
            400,
            "chat_invalid",
        ),
        (
            "POST",
            "/chat/media/delete",
            Some(json!({"all": true})),
            200,
            "",
        ),
        (
            "POST",
            "/chat/media/delete",
            Some(json!({"ids": [id]})),
            200,
            "",
        ),
        (
            "POST",
            "/chat/media/delete",
            Some(json!({})),
            400,
            "chat_invalid",
        ),
        (
            "POST",
            "/chat/media/delete",
            Some(json!({"ids": [id], "all": true})),
            400,
            "chat_invalid",
        ),
        (
            "POST",
            "/chat/media/delete",
            Some(json!({"ids": ids(501)})),
            400,
            "chat_invalid",
        ),
        (
            "POST",
            "/chat/media/restore",
            Some(json!({"all": true})),
            200,
            "",
        ),
        (
            "POST",
            "/chat/media/restore",
            Some(json!({})),
            400,
            "chat_invalid",
        ),
    ];
    for (method, uri, body, status, error) in cases {
        let answer = call(chat.clone(), &who, method, uri, body.clone()).await;
        assert_eq!(
            code(&answer),
            (status, error.to_owned()),
            "{method} {uri} {body:?}"
        );
    }
    let commit = call(
        chat.clone(),
        &who,
        "POST",
        &format!("/chat/media/{id}/commit"),
        None,
    )
    .await;
    assert_eq!(code(&commit), (404, "chat_not_found".to_owned()));
    let bad = call(chat, &who, "POST", "/chat/media/not-a-uuid/commit", None).await;
    assert_eq!(code(&bad), (404, "chat_not_found".to_owned()));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn urls_are_limited_to_120_calls_a_minute(pool: PgPool) {
    let chat = chat_state(pool, redis(), true);
    let who = fresh_caller();
    let body = json!({"ids": [Uuid::new_v4()]});
    for _ in 0..120 {
        let answer = call(
            chat.clone(),
            &who,
            "POST",
            "/chat/media/urls",
            Some(body.clone()),
        )
        .await;
        assert_eq!(answer.0, StatusCode::OK);
    }
    let limited = call(chat, &who, "POST", "/chat/media/urls", Some(body)).await;
    assert_eq!(code(&limited), (429, "chat_read_limited".to_owned()));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_config_view_shows_uploads_off_the_star_state_and_the_used_bytes(pool: PgPool) {
    let chat = chat_state(pool.clone(), redis(), true);
    let who = fresh_caller();
    let off = view::config(&chat, &who.user_id).await.expect("a view");
    assert!(!off.uploads);
    assert!(!off.star);
    assert_eq!((off.quota, off.used), (1_073_741_824, 0));
    assert_eq!(off.limits, view::limits());
    sqlx::query("INSERT INTO chat.usage (user_id, bytes, reserved) VALUES ($1, 700, 30)")
        .bind(&who.user_id)
        .execute(&pool)
        .await
        .expect("a usage row");
    SubscriptionsService::new(pool.clone(), false)
        .upsert(&who.user_id, FAR, Some(1))
        .await
        .expect("a subscription");
    let chat = chat_state(pool, redis(), true);
    let after = view::config(&chat, &who.user_id).await.expect("a view");
    assert!(after.star);
    assert_eq!(after.used, 730);
    assert!(!after.uploads);
}

#[tokio::test]
async fn the_media_routes_are_never_mounted_on_the_public_router() {
    let router: axum::Router = crate::modules::chat::routes::public(offline_chat_state(true));
    for (method, uri) in [
        ("POST", "/chat/media/reserve"),
        ("POST", "/chat/media/urls"),
        ("GET", "/chat/media/usage"),
        ("GET", "/chat/media/mine"),
        ("POST", "/chat/media/delete"),
        ("POST", "/chat/media/restore"),
    ] {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .expect("a request");
        request.extensions_mut().insert(caller());
        let status = router
            .clone()
            .oneshot(request)
            .await
            .expect("an answer")
            .status();
        assert!(
            status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED,
            "{method} {uri} answered {status}"
        );
    }
}
