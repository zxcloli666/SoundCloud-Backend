use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::super::router_search_tests::{soundcloud_hits, state_with};
use super::{session_for, tuned};
use crate::modules::subscriptions::SubscriptionsService;
use crate::modules::voice_speak::testing::{Answer, CLIP, Speaking, speaking};

struct Heard {
    status: StatusCode,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl Heard {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

fn fresh_user() -> String {
    (Uuid::now_v7().as_u128() % 1_000_000_000_000_000).to_string()
}

fn line() -> String {
    json!({"voice": "loli", "text": "Привет.", "keep": true}).to_string()
}

async fn speak_app(pool: &PgPool, star: bool) -> anyhow::Result<(axum::Router, Speaking)> {
    let mut state = state_with(pool, soundcloud_hits(), tuned(|_| {})).await?;
    if !star {
        state.subscriptions = SubscriptionsService::new(pool.clone(), false);
    }
    let speaking = speaking(Answer::Clip, 4);
    state.voice_speak = speaking.speak.clone();
    Ok((crate::router::build(state), speaking))
}

async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    session: Option<Uuid>,
    body: Option<String>,
) -> anyhow::Result<Heard> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(session) = session {
        request = request.header("x-session-id", session.to_string());
    }
    let response = app
        .clone()
        .oneshot(request.body(body.map_or_else(Body::empty, Body::from))?)
        .await?;
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = to_bytes(response.into_body(), usize::MAX).await?.to_vec();
    Ok(Heard {
        status,
        content_type,
        body,
    })
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_listener_without_star_is_refused_before_the_line_is_read(
    pool: PgPool,
) -> anyhow::Result<()> {
    let session = session_for(&pool, &fresh_user()).await?;
    let (app, speaking) = speak_app(&pool, false).await?;

    for body in [line(), "{not json".to_owned()] {
        let heard = call(&app, "POST", "/voice/speak", Some(session), Some(body)).await?;
        assert_eq!(heard.status, StatusCode::FORBIDDEN);
        assert_eq!(heard.json()["code"], "star_required");
    }
    assert!(speaking.voice.asked().is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_listener_with_star_gets_a_ticket_then_the_clip_then_the_kept_phrase(
    pool: PgPool,
) -> anyhow::Result<()> {
    let session = session_for(&pool, &fresh_user()).await?;
    let (app, speaking) = speak_app(&pool, true).await?;

    let posted = call(&app, "POST", "/voice/speak", Some(session), Some(line())).await?;
    assert_eq!(posted.status, StatusCode::ACCEPTED);
    let ticket = posted.json()["id"].as_str().unwrap_or_default().to_owned();
    let polled = call(
        &app,
        "GET",
        &format!("/voice/speak/{ticket}"),
        Some(session),
        None,
    )
    .await?;
    speaking.settled().await;
    let cached = call(&app, "POST", "/voice/speak", Some(session), Some(line())).await?;

    assert_eq!(polled.status, StatusCode::OK);
    assert_eq!(polled.content_type.as_deref(), Some("audio/ogg"));
    assert_eq!(polled.body, CLIP);
    assert_eq!(cached.status, StatusCode::OK);
    assert_eq!(cached.content_type.as_deref(), Some("audio/ogg"));
    assert_eq!(cached.body, CLIP);
    assert_eq!(speaking.voice.asked().len(), 1);
    let unknown = call(&app, "GET", "/voice/speak/nope", Some(session), None).await?;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    let anonymous = call(&app, "POST", "/voice/speak", None, Some(line())).await?;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    Ok(())
}
