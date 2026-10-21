use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::auth::token_matches;
use super::reports::{AdminDecision, Resolve, apply, detail};
use super::rewrap::rewrap_with;
use super::router;
use crate::modules::chat::ChatState;
use crate::modules::chat::admin::audit;
use crate::modules::chat::moderation::testing::{
    ADMIN_TOKEN, TestKeys, audit_actions, body_of, conversation, conversation_key, post, state,
};
use crate::modules::chat::reports::{FileReport, file_with, load};
use crate::modules::chat::test_support::offline_chat_state;
use crate::modules::chat::users;

async fn call(
    chat: &Arc<ChatState>,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value, String) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("x-chat-admin-user", "owner");
    if let Some(token) = token {
        request = request.header("x-chat-admin-token", token);
    }
    if body.is_some() {
        request = request.header(header::CONTENT_TYPE, "application/json");
    }
    let request = request
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
        .expect("a request");
    let app: axum::Router = router(chat.clone());
    let response = app.oneshot(request).await.expect("an answer");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("a body");
    let text = String::from_utf8_lossy(&bytes).into_owned();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        text,
    )
}

#[test]
fn the_token_check_compares_digests_and_fails_closed() {
    assert!(token_matches(Some("abc"), Some("abc")));
    assert!(!token_matches(Some("abc"), Some("abd")));
    assert!(!token_matches(Some("abc"), Some("abcd")));
    assert!(!token_matches(Some("abc"), None));
    assert!(!token_matches(None, None));
    assert!(!token_matches(None, Some("")));
}

#[tokio::test]
async fn every_admin_route_refuses_a_missing_or_wrong_token() {
    let missing_file = offline_chat_state(true);
    let with_file = state(
        sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://scd@127.0.0.1:1/none")
            .expect("a lazy pool"),
        None,
    );
    let conversation = Uuid::now_v7();
    let short = &ADMIN_TOKEN[..ADMIN_TOKEN.len() - 1];
    for (method, uri) in [
        ("GET", "/chat-admin/reports".to_owned()),
        ("GET", format!("/chat-admin/reports/{conversation}")),
        ("POST", format!("/chat-admin/reports/{conversation}")),
        ("GET", "/chat-admin/restrictions".to_owned()),
        ("POST", "/chat-admin/restrictions".to_owned()),
        ("DELETE", format!("/chat-admin/messages/{conversation}/5")),
        ("POST", "/chat-admin/unlist/ABC123".to_owned()),
        ("POST", "/chat-admin/rewrap".to_owned()),
        ("GET", "/chat-admin/stats".to_owned()),
        ("GET", "/chat-admin/metrics".to_owned()),
        ("POST", "/chat-admin/loadtest/seed".to_owned()),
        ("POST", "/chat-admin/loadtest/purge".to_owned()),
    ] {
        for (chat, token) in [
            (&missing_file, Some(ADMIN_TOKEN)),
            (&with_file, None),
            (&with_file, Some("wrong")),
            (&with_file, Some(short)),
        ] {
            let (status, body, _) = call(chat, method, &uri, token, None).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {uri}");
            assert_eq!(body, json!({"error": "chat_admin_unauthorized"}));
        }
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn metrics_and_stats_answer_with_the_token(pool: PgPool) {
    crate::metrics::init();
    let chat = state(pool, None);
    let (status, _, text) =
        call(&chat, "GET", "/chat-admin/metrics", Some(ADMIN_TOKEN), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(text.contains("api_pg_pool_connections"), "{text}");

    let (status, stats, _) = call(&chat, "GET", "/chat-admin/stats", Some(ADMIN_TOKEN), None).await;
    assert_eq!(status, StatusCode::OK, "{stats}");
    assert_eq!(stats["reports_open"], 0);
    assert_eq!(stats["ai_enabled"], false);

    let bearer = Request::get("/chat-admin/stats")
        .header(header::AUTHORIZATION, format!("Bearer {ADMIN_TOKEN}"))
        .body(Body::empty())
        .expect("a request");
    let app: axum::Router = router(chat.clone());
    assert_eq!(
        app.oneshot(bearer).await.expect("an answer").status(),
        StatusCode::OK
    );
}

async fn filed(pool: &PgPool, chat: &Arc<ChatState>, keys: &TestKeys) -> (Uuid, Uuid, i64) {
    let dm = conversation(pool, "direct", &[("71", true), ("72", true)]).await;
    let ck = conversation_key(pool, keys, dm).await;
    post(pool, dm, &ck, "71", "hi").await;
    let seq = post(pool, dm, &ck, "72", "leaked address 5 Main st").await;
    let report = file_with(
        chat,
        keys,
        "71",
        &FileReport {
            conversation_id: dm,
            seq_from: seq,
            seq_to: seq,
            reason: "doxxing".to_owned(),
            comment: None,
        },
    )
    .await
    .expect("a report");
    (report.id, dm, seq)
}

fn resolve(decision: AdminDecision) -> Resolve {
    Resolve {
        decision,
        delete_seqs: Vec::new(),
        ban_days: None,
        ban_permanent: false,
        lift_restriction: false,
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn the_admin_view_decrypts_only_the_report_and_writes_an_audit_row(pool: PgPool) {
    let keys = TestKeys::default();
    let chat = state(pool.clone(), None);
    let (id, _, seq) = filed(&pool, &chat, &keys).await;

    let (status, listed, _) =
        call(&chat, "GET", "/chat-admin/reports", Some(ADMIN_TOKEN), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed.as_array().map(Vec::len), Some(1));
    assert_eq!(listed[0]["id"], id.to_string());
    assert!(listed[0].get("evidence").is_none());
    assert!(audit_actions(&pool, &audit::report(id)).await.is_empty());

    let shown = detail(&chat, &keys, "owner", id).await.expect("a detail");
    let texts: Vec<(&str, bool)> = shown
        .lines
        .iter()
        .map(|line| (line.text.as_str(), line.reported))
        .collect();
    assert_eq!(
        texts,
        vec![("hi", false), ("leaked address 5 Main st", true)]
    );
    assert_eq!(shown.lines[1].seq, seq);
    let rows = audit_actions(&pool, &audit::report(id)).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].0.as_str(), rows[0].1.as_str()),
        ("owner", "decrypt")
    );
    assert_eq!(shown.audit.len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
async fn the_owner_overrides_a_report_and_every_step_is_audited(pool: PgPool) {
    let keys = TestKeys::default();
    let chat = state(pool.clone(), None);
    let (id, dm, seq) = filed(&pool, &chat, &keys).await;

    let mut uphold = resolve(AdminDecision::Uphold);
    uphold.delete_seqs = vec![seq];
    uphold.ban_days = Some(3);
    let upheld = apply(&chat, "owner", id, &uphold).await.expect("upheld");
    assert_eq!(upheld.state, "actioned");
    assert_eq!(upheld.handled_by.as_deref(), Some("owner"));
    assert!(body_of(&pool, dm, seq).await.is_none());
    assert!(
        users::restriction(&chat, "72")
            .await
            .expect("read")
            .is_some()
    );

    let mut reject = resolve(AdminDecision::Reject);
    reject.lift_restriction = true;
    assert_eq!(
        apply(&chat, "owner", id, &reject)
            .await
            .expect("rejected")
            .state,
        "rejected"
    );
    assert!(
        users::restriction(&chat, "72")
            .await
            .expect("read")
            .is_none()
    );

    assert_eq!(
        apply(&chat, "owner", id, &resolve(AdminDecision::Reopen))
            .await
            .expect("reopened")
            .state,
        "open"
    );
    let row = load(&chat, id).await.expect("read").expect("a row");
    assert!(row.evidence.is_some());

    let report_rows = audit_actions(&pool, &audit::report(id)).await;
    let actions: Vec<&str> = report_rows.iter().map(|row| row.1.as_str()).collect();
    assert_eq!(
        actions,
        vec!["delete_messages", "actioned", "rejected", "open"]
    );
    let user_rows = audit_actions(&pool, &audit::user("72")).await;
    let actions: Vec<&str> = user_rows.iter().map(|row| row.1.as_str()).collect();
    assert_eq!(actions, vec!["ban", "lift"]);
    assert!(user_rows.iter().all(|row| row.0 == "owner"));

    let mut outside = resolve(AdminDecision::Uphold);
    outside.delete_seqs = vec![seq + 5];
    assert!(apply(&chat, "owner", id, &outside).await.is_err());
    let mut mixed = resolve(AdminDecision::Reject);
    mixed.ban_permanent = true;
    assert!(apply(&chat, "owner", id, &mixed).await.is_err());
    let mut undone = resolve(AdminDecision::Uphold);
    undone.ban_days = Some(1);
    undone.lift_restriction = true;
    assert!(apply(&chat, "owner", id, &undone).await.is_err());

    let reopened_until: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT purge_after FROM chat.reports WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("a purge date");
    assert!(
        reopened_until
            .is_some_and(|until| until > chrono::Utc::now() + chrono::Duration::days(179))
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn restrictions_and_message_deletes_go_through_the_admin_routes(pool: PgPool) {
    let keys = TestKeys::default();
    let chat = state(pool.clone(), None);
    let dm = conversation(&pool, "direct", &[("81", true), ("82", true)]).await;
    let ck = conversation_key(&pool, &keys, dm).await;
    let seq = post(&pool, dm, &ck, "82", "bad").await;

    let (status, answer, _) = call(
        &chat,
        "POST",
        "/chat-admin/restrictions",
        Some(ADMIN_TOKEN),
        Some(json!({"user_id": "82", "scope": ["dm_new"], "hours": 2})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["scope"], json!(["dm_new"]));
    let (_, listed, _) = call(
        &chat,
        "GET",
        "/chat-admin/restrictions",
        Some(ADMIN_TOKEN),
        None,
    )
    .await;
    assert_eq!(listed[0]["user_id"], "82");
    let (status, _, _) = call(
        &chat,
        "POST",
        "/chat-admin/restrictions",
        Some(ADMIN_TOKEN),
        Some(json!({"user_id": "82", "scope": ["everything"], "hours": 2})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = call(
        &chat,
        "POST",
        "/chat-admin/restrictions",
        Some(ADMIN_TOKEN),
        Some(json!({"user_id": "82", "lift": true})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        users::restriction(&chat, "82")
            .await
            .expect("read")
            .is_none()
    );

    let uri = format!("/chat-admin/messages/{dm}/{seq}");
    let (status, _, _) = call(&chat, "DELETE", &uri, Some(ADMIN_TOKEN), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(body_of(&pool, dm, seq).await.is_none());
    let (status, _, _) = call(&chat, "DELETE", &uri, Some(ADMIN_TOKEN), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let rows = audit_actions(&pool, &audit::message(dm, seq)).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].0.as_str(), rows[0].1.as_str()),
        ("owner", "delete_message")
    );
    assert_eq!(audit_actions(&pool, &audit::user("82")).await.len(), 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn rewrap_moves_keys_and_evidence_to_the_current_versions(pool: PgPool) {
    let old = TestKeys::default();
    let chat = state(pool.clone(), None);
    let (id, dm, _) = filed(&pool, &chat, &old).await;
    let fresh = TestKeys {
        kek_current: 2,
        reports_current: 3,
    };

    let first = rewrap_with(&chat, &fresh, "owner").await.expect("a pass");
    assert_eq!((first.keys, first.evidence), (1, 1));
    let again = rewrap_with(&chat, &fresh, "owner").await.expect("a pass");
    assert_eq!((again.keys, again.evidence), (0, 0));

    let version: i16 = sqlx::query_scalar(
        "SELECT kek_version FROM chat.conversation_keys WHERE conversation_id = $1",
    )
    .bind(dm)
    .fetch_one(&pool)
    .await
    .expect("a key row");
    assert_eq!(version, 2);
    let shown = detail(&chat, &fresh, "owner", id).await.expect("a detail");
    assert_eq!(shown.report.id, id);
    assert_eq!(shown.lines.len(), 2);
    assert_eq!(audit_actions(&pool, "keys").await.len(), 1);
}
