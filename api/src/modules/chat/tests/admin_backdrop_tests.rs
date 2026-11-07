use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use sqlx::PgPool;
use tower::ServiceExt;

use super::media_fixtures::{decision, get, group_backdrop, group_report, room_backdrop};
use super::media_support::status_of;
use super::resolve::{AdminDecision, apply};
use super::restore::restore_with;
use super::router;
use crate::modules::chat::ChatError;
use crate::modules::chat::admin::audit;
use crate::modules::chat::moderation::testing::{ADMIN_TOKEN, audit_actions, state};

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_backdrop_report_shows_its_kind_media_and_one_room_meta_line(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let (_, report, backdrop) = room_backdrop(&pool, &chat).await;

    let (status, body) = get(&chat, &format!("/chat-admin/reports/{report}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["report"]["kind"], "backdrop");
    assert_eq!(body["report"]["media_id"], backdrop.id.to_string());
    assert_eq!(body["lines"].as_array().map(Vec::len), Some(1));
    assert_eq!(body["lines"][0]["class"], "meta");
    assert_eq!(
        body["lines"][0]["media"],
        json!([{"id": backdrop.id, "kind": "backdrop", "w": 1920, "h": 1080, "size": backdrop.bytes()}])
    );
    let (_, listed) = get(&chat, "/chat-admin/reports").await;
    assert_eq!(listed[0]["kind"], "backdrop");
    assert_eq!(listed[0]["media_id"], backdrop.id.to_string());
    let (_, trail) = get(&chat, &format!("/chat-admin/reports/{report}/audit")).await;
    assert_eq!(trail["report"]["kind"], "backdrop");
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_group_backdrop_report_finds_the_image_named_in_the_backdrop_slot(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let (_, report, avatar, backdrop) = group_backdrop(&pool, &chat).await;

    let (status, body) = get(&chat, &format!("/chat-admin/reports/{report}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["report"]["kind"], "backdrop");
    assert_eq!(body["report"]["media_id"], backdrop.id.to_string());
    assert_eq!(body["lines"].as_array().map(Vec::len), Some(1));
    assert_eq!(body["lines"][0]["class"], "meta");
    assert_eq!(
        body["lines"][0]["media"],
        json!([
            {"id": avatar.id, "kind": "avatar", "w": 512, "h": 512, "size": avatar.bytes()},
            {"id": backdrop.id, "kind": "backdrop", "w": 1920, "h": 1080, "size": backdrop.bytes()}
        ])
    );
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn clear_backdrop_needs_an_upheld_backdrop_report(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let keys = &chat.keks;
    let (_, backdrop_report, _) = room_backdrop(&pool, &chat).await;
    let posted = group_report(&pool, &chat).await;

    let mut clear = decision(AdminDecision::Uphold);
    clear.clear_backdrop = true;
    assert_eq!(
        apply(&chat, &chat.keks, "owner", posted.report, &clear).await,
        Err(ChatError::Invalid)
    );
    for refused in [AdminDecision::Reject, AdminDecision::Reopen] {
        let mut input = decision(refused);
        input.clear_backdrop = true;
        assert_eq!(
            apply(&chat, &chat.keks, "owner", backdrop_report, &input).await,
            Err(ChatError::Invalid)
        );
    }
    let mut seqs = decision(AdminDecision::Uphold);
    seqs.delete_seqs = vec![1];
    assert_eq!(
        apply(&chat, &chat.keks, "owner", backdrop_report, &seqs).await,
        Err(ChatError::Invalid)
    );
    assert_eq!(
        restore_with(&chat, keys, "owner", backdrop_report, &[1]).await,
        Err(ChatError::Invalid)
    );
    let request = Request::post(format!("/chat-admin/reports/{backdrop_report}"))
        .header("x-chat-admin-token", ADMIN_TOKEN)
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"decision": "reject", "clear_backdrop": true}).to_string(),
        ))
        .expect("a request");
    let app: axum::Router = router(chat.clone());
    let response = app.oneshot(request).await.expect("an answer");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let state: String = sqlx::query_scalar("SELECT state FROM chat.reports WHERE id = $1")
        .bind(backdrop_report)
        .fetch_one(&pool)
        .await
        .expect("a state");
    assert_eq!(state, "open");
    assert!(
        audit_actions(&pool, &audit::report(backdrop_report))
            .await
            .is_empty()
    );
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_failing_room_gate_leaves_the_backdrop_report_untouched(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let (_, report, backdrop) = room_backdrop(&pool, &chat).await;

    let mut clear = decision(AdminDecision::Uphold);
    clear.clear_backdrop = true;
    clear.ban_days = Some(1);
    let refused = apply(&chat, &chat.keks, "owner", report, &clear).await;
    assert!(refused.is_err(), "{refused:?}");
    let state: String = sqlx::query_scalar("SELECT state FROM chat.reports WHERE id = $1")
        .bind(report)
        .fetch_one(&pool)
        .await
        .expect("a state");
    assert_eq!(state, "open");
    assert!(
        audit_actions(&pool, &audit::report(report))
            .await
            .is_empty()
    );
    assert!(audit_actions(&pool, &audit::user("61")).await.is_empty());
    assert_eq!(
        status_of(&pool, backdrop.id).await.as_deref(),
        Some("attached")
    );

    let upheld = apply(
        &chat,
        &chat.keks,
        "owner",
        report,
        &decision(AdminDecision::Uphold),
    )
    .await
    .expect("an uphold without a clear");
    assert_eq!(upheld.state, "actioned");
    assert_eq!(upheld.media_id, Some(backdrop.id));
}
