use axum::body::Body;
use axum::http::{Request, StatusCode};
use sc_chat_wire::msg::Class;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::cipher::Role;
use super::evidence::MediaKind;
use super::filed::ReportKind;
use super::media::{view_with, viewable};
use super::media_fixtures::{attached, get, group_report, set_status, view_rows, with_store};
use super::media_support::{Upload, conversation_key, image, post_envelope, send, single, webp};
use super::reports::detail;
use super::router;
use crate::modules::chat::ChatError;
use crate::modules::chat::moderation::testing::{ADMIN_TOKEN, conversation, state};
use crate::modules::chat::reports::{FileReport, file_with};

type ShownMedia = (Uuid, MediaKind, u32, u32, u64);

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_detail_lists_post_images_and_the_group_avatar_without_thumbs(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let keys = &chat.keks;
    let posted = group_report(&pool, &chat).await;

    let shown = detail(&chat, keys, "owner", posted.report)
        .await
        .expect("a detail");
    assert_eq!(shown.report.kind, ReportKind::Messages);
    assert_eq!(shown.report.media_id, None);
    let media: Vec<(i64, Vec<ShownMedia>)> = shown
        .lines
        .iter()
        .map(|line| {
            let media = line
                .media
                .iter()
                .map(|one| (one.id, one.kind, one.w, one.h, one.size))
                .collect();
            (line.line.seq, media)
        })
        .collect();
    let (first, first_thumb) = &posted.images[0];
    let (second, _) = &posted.images[1];
    assert_eq!(media.len(), 3);
    assert!(media[0].1.is_empty());
    assert_eq!(
        media[1],
        (
            posted.meta_seq,
            vec![(
                posted.avatar.id,
                MediaKind::Avatar,
                512,
                512,
                posted.avatar.bytes() as u64
            )]
        )
    );
    assert_eq!(
        media[2],
        (
            posted.post_seq,
            vec![
                (first.id, MediaKind::Image, 1600, 900, first.bytes() as u64),
                (
                    second.id,
                    MediaKind::Image,
                    1600,
                    900,
                    second.bytes() as u64
                ),
            ]
        )
    );
    assert!(
        media
            .iter()
            .all(|(_, media)| media.iter().all(|one| one.0 != first_thumb.id))
    );

    let (status, body) = get(&chat, &format!("/chat-admin/reports/{}", posted.report)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["report"]["kind"], "messages");
    assert_eq!(body["report"]["media_id"], Value::Null);
    let line = &body["lines"][2];
    assert_eq!(line["seq"], posted.post_seq);
    assert_eq!(line["text"], "bad pictures [2 image(s)]");
    assert_eq!(line["reported"], true);
    assert_eq!(
        line["media"][0],
        json!({"id": first.id, "kind": "image", "w": 1600, "h": 900, "size": first.bytes()})
    );
    assert_eq!(body["lines"][0]["media"], json!([]));

    let (status, listed) = get(&chat, "/chat-admin/reports").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed[0]["kind"], "messages");
    assert_eq!(listed[0]["media_id"], Value::Null);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_media_id_outside_the_evidence_is_not_found_and_not_audited(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let keys = &chat.keks;
    let posted = group_report(&pool, &chat).await;
    let elsewhere = Upload::new(Role::Image, &webp("other", 100));
    attached(&pool, &elsewhere, None, "92", posted.group).await;

    for media in [elsewhere.id, Uuid::new_v4()] {
        assert_eq!(
            view_with(&chat, keys, "owner", posted.report, media).await,
            Err(ChatError::NotFound)
        );
    }
    assert_eq!(
        view_with(&chat, keys, "owner", Uuid::now_v7(), posted.avatar.id).await,
        Err(ChatError::NotFound)
    );
    let (status, body) = get(
        &chat,
        &format!(
            "/chat-admin/reports/{}/media/{}",
            posted.report, elsewhere.id
        ),
    )
    .await;
    assert_eq!(
        (status, body),
        (StatusCode::NOT_FOUND, json!({"error": "chat_not_found"}))
    );
    sqlx::query(
        "UPDATE chat.reports SET evidence = NULL, evidence_kek_version = NULL WHERE id = $1",
    )
    .bind(posted.report)
    .execute(&pool)
    .await
    .expect("evidence dropped");
    assert_eq!(
        view_with(&chat, keys, "owner", posted.report, posted.avatar.id).await,
        Err(ChatError::NotFound)
    );
    assert!(view_rows(&pool, posted.report).await.is_empty());
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_row_that_is_gone_lost_reserved_or_deleting_past_use_answers_gone(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let keys = &chat.keks;
    let posted = group_report(&pool, &chat).await;
    let (first, first_thumb) = &posted.images[0];
    let (second, second_thumb) = &posted.images[1];

    sqlx::query("DELETE FROM chat.media WHERE id = $1 OR parent_id = $1")
        .bind(first.id)
        .execute(&pool)
        .await
        .expect("rows gone");
    set_status(&pool, second.id, "lost").await;
    set_status(&pool, posted.avatar.id, "deleting").await;
    for media in [
        first.id,
        first_thumb.id,
        second.id,
        second_thumb.id,
        posted.avatar.id,
    ] {
        assert_eq!(
            view_with(&chat, keys, "owner", posted.report, media).await,
            Err(ChatError::MediaGone),
            "{media}"
        );
    }
    set_status(&pool, second.id, "reserved").await;
    assert_eq!(
        view_with(&chat, keys, "owner", posted.report, second_thumb.id).await,
        Err(ChatError::MediaGone)
    );
    sqlx::query(
        "UPDATE chat.media SET restorable_until = now() - interval '1 second' WHERE id = $1",
    )
    .bind(posted.avatar.id)
    .execute(&pool)
    .await
    .expect("a passed window");
    assert_eq!(
        view_with(&chat, keys, "owner", posted.report, posted.avatar.id).await,
        Err(ChatError::MediaGone)
    );
    let (status, body) = get(
        &chat,
        &format!("/chat-admin/reports/{}/media/{}", posted.report, second.id),
    )
    .await;
    assert_eq!(
        (status, body),
        (StatusCode::GONE, json!({"error": "chat_media_gone"}))
    );
    assert!(view_rows(&pool, posted.report).await.is_empty());
}

#[test]
fn only_live_held_or_still_restorable_rows_are_viewable() {
    for status in ["uploaded", "attached", "held"] {
        assert!(viewable(status, false, false), "{status}");
    }
    assert!(viewable("deleting", false, true));
    assert!(viewable("deleting", true, false));
    assert!(!viewable("deleting", false, false));
    for status in ["reserved", "lost", "unknown"] {
        assert!(!viewable(status, true, true), "{status}");
    }
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_unreachable_store_is_busy_and_writes_no_view_row(pool: PgPool) {
    let chat = with_store(
        state(pool.clone(), None),
        "http://127.0.0.1:1",
        "key",
        "secret",
    );
    let keys = &chat.keks;
    let posted = group_report(&pool, &chat).await;
    let answer = view_with(&chat, keys, "owner", posted.report, posted.avatar.id).await;
    assert!(matches!(answer, Err(ChatError::Busy { .. })), "{answer:?}");
    assert!(view_rows(&pool, posted.report).await.is_empty());
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn attachments_core_would_not_show_are_neither_listed_nor_served(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let keys = &chat.keks;
    let dm = conversation(&pool, "direct", &[("51", true), ("52", true)]).await;
    let ck = conversation_key(&pool, keys, dm).await;
    let pair = |tag: &str| {
        (
            Upload::new(Role::Image, &webp(tag, 3_000)),
            Upload::new(Role::Thumb, &webp(tag, 200)),
        )
    };
    let (good, good_thumb) = pair("good");
    let (unhashed, unhashed_thumb) = pair("unhashed");
    let (thumbless, _) = pair("thumbless");
    let mut no_hash = image(&unhashed, &unhashed_thumb);
    no_hash.sha256_cipher.clear();
    let no_thumb = single(&thumbless, 800, 600);
    let seq = send(
        &pool,
        dm,
        &ck,
        "52",
        Class::Post,
        &post_envelope("mixed", vec![no_hash, image(&good, &good_thumb), no_thumb]),
    )
    .await
    .seq;
    let report = file_with(
        &chat,
        keys,
        "51",
        &FileReport {
            conversation_id: dm,
            seq_from: seq,
            seq_to: seq,
            reason: "spam".to_owned(),
            comment: None,
        },
    )
    .await
    .expect("a report")
    .id;

    let shown = detail(&chat, keys, "owner", report)
        .await
        .expect("a detail");
    let listed: Vec<Uuid> = shown.lines[0].media.iter().map(|one| one.id).collect();
    assert_eq!(listed, vec![good.id]);
    for media in [unhashed.id, unhashed_thumb.id, thumbless.id] {
        assert_eq!(
            view_with(&chat, keys, "owner", report, media).await,
            Err(ChatError::NotFound)
        );
    }
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_media_route_answers_only_get(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let posted = group_report(&pool, &chat).await;
    let request = Request::head(format!(
        "/chat-admin/reports/{}/media/{}",
        posted.report, posted.avatar.id
    ))
    .header("x-chat-admin-token", ADMIN_TOKEN)
    .body(Body::empty())
    .expect("a request");
    let app: axum::Router = router(chat.clone());
    let response = app.oneshot(request).await.expect("an answer");
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    let anonymous = Request::head(format!(
        "/chat-admin/reports/{}/media/{}",
        posted.report, posted.avatar.id
    ))
    .body(Body::empty())
    .expect("a request");
    let app: axum::Router = router(chat.clone());
    let response = app.oneshot(anonymous).await.expect("an answer");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(view_rows(&pool, posted.report).await.is_empty());
}
