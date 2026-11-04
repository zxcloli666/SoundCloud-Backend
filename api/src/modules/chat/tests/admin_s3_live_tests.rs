use std::sync::Arc;

use aws_sdk_s3::Client;
use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::media::view_with;
use super::media_fixtures::{group_report, room_backdrop, set_status, with_bucket};
use super::media_support::{Upload, hold};
use super::router;
use crate::modules::chat::admin::audit;
use crate::modules::chat::moderation::testing::{ADMIN_TOKEN, audit_actions, state};
use crate::modules::chat::{ChatError, ChatState};

struct Live {
    endpoint: String,
    access: String,
    secret: String,
}

fn live() -> Live {
    let var = |name: &str| {
        std::env::var(name).unwrap_or_else(|_| panic!("{name} must point these tests at SeaweedFS"))
    };
    Live {
        endpoint: var("CHAT_TEST_S3_ENDPOINT"),
        access: var("CHAT_TEST_S3_ACCESS_KEY"),
        secret: var("CHAT_TEST_S3_SECRET_KEY"),
    }
}

async fn bucketed(pool: &PgPool) -> Arc<ChatState> {
    let live = live();
    let bucket = format!("admin-{}", Uuid::new_v4().simple());
    let client = Client::from_conf(
        aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(&live.endpoint)
            .credentials_provider(Credentials::new(
                &live.access,
                &live.secret,
                None,
                None,
                "admin-tests",
            ))
            .force_path_style(true)
            .build(),
    );
    client
        .create_bucket()
        .bucket(&bucket)
        .send()
        .await
        .expect("a bucket");
    with_bucket(
        state(pool.clone(), None),
        &live.endpoint,
        &live.access,
        &live.secret,
        &bucket,
    )
}

async fn stored(chat: &ChatState, upload: &Upload) {
    chat.s3
        .put(1, &upload.object_key(), upload.ciphertext.clone())
        .await
        .expect("an object");
}

async fn fetch(
    chat: &Arc<ChatState>,
    report: Uuid,
    media: Uuid,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let request = Request::get(format!("/chat-admin/reports/{report}/media/{media}"))
        .header("x-chat-admin-token", ADMIN_TOKEN)
        .header("x-chat-admin-user", "owner")
        .body(Body::empty())
        .expect("a request");
    let app: axum::Router = router(chat.clone());
    let response = app.oneshot(request).await.expect("an answer");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("a body");
    (status, headers, bytes.to_vec())
}

async fn views(pool: &PgPool, report: Uuid) -> Vec<Value> {
    audit_actions(pool, &audit::report(report))
        .await
        .into_iter()
        .filter(|row| row.0 == "owner" && row.1 == "view_media")
        .map(|row| row.2)
        .collect()
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs SeaweedFS at CHAT_TEST_S3_ENDPOINT"]
async fn an_image_and_its_thumb_come_back_as_their_exact_plaintext(pool: PgPool) {
    let chat = bucketed(&pool).await;
    let keys = &chat.keks;
    let posted = group_report(&pool, &chat).await;
    let (full, thumb) = &posted.images[0];
    for upload in [full, thumb] {
        stored(&chat, upload).await;
    }

    let image = view_with(&chat, keys, "owner", posted.report, full.id)
        .await
        .expect("the image");
    assert_eq!(image, full.plain);
    let small = view_with(&chat, keys, "owner", posted.report, thumb.id)
        .await
        .expect("the thumb");
    assert_eq!(small, thumb.plain);
    assert_eq!(
        views(&pool, posted.report).await,
        vec![json!({"media": full.id}), json!({"media": thumb.id})]
    );
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs SeaweedFS at CHAT_TEST_S3_ENDPOINT"]
async fn the_route_answers_webp_with_no_store_and_nosniff(pool: PgPool) {
    let chat = bucketed(&pool).await;
    let posted = group_report(&pool, &chat).await;
    stored(&chat, &posted.avatar).await;
    let (status, headers, body) = fetch(&chat, posted.report, posted.avatar.id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CONTENT_TYPE], "image/webp");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(body, posted.avatar.plain);
    assert_eq!(
        views(&pool, posted.report).await,
        vec![json!({"media": posted.avatar.id})]
    );
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs SeaweedFS at CHAT_TEST_S3_ENDPOINT"]
async fn held_and_restorable_rows_are_served_and_a_tampered_object_is_gone(pool: PgPool) {
    let chat = bucketed(&pool).await;
    let keys = &chat.keks;
    let posted = group_report(&pool, &chat).await;
    let (first, first_thumb) = &posted.images[0];
    let (second, second_thumb) = &posted.images[1];
    for upload in [first, first_thumb, second, second_thumb, &posted.avatar] {
        stored(&chat, upload).await;
    }

    set_status(&pool, first.id, "held").await;
    assert_eq!(
        view_with(&chat, keys, "owner", posted.report, first_thumb.id).await,
        Ok(first_thumb.plain.clone())
    );
    set_status(&pool, first.id, "deleting").await;
    assert_eq!(
        view_with(&chat, keys, "owner", posted.report, first.id).await,
        Err(ChatError::MediaGone)
    );
    hold(&pool, first.id, posted.report).await;
    assert_eq!(
        view_with(&chat, keys, "owner", posted.report, first.id).await,
        Ok(first.plain.clone())
    );
    set_status(&pool, posted.avatar.id, "deleting").await;
    sqlx::query(
        "UPDATE chat.media SET restorable_until = now() + interval '15 seconds' WHERE id = $1",
    )
    .bind(posted.avatar.id)
    .execute(&pool)
    .await
    .expect("an open window");
    assert_eq!(
        view_with(&chat, keys, "owner", posted.report, posted.avatar.id).await,
        Ok(posted.avatar.plain.clone())
    );
    let served = views(&pool, posted.report).await.len();
    assert_eq!(served, 3);

    let mut tampered = second.ciphertext.clone();
    tampered[100] ^= 0x01;
    chat.s3
        .put(1, &second.object_key(), tampered)
        .await
        .expect("a swapped object");
    let mut tampered_thumb = second_thumb.ciphertext.clone();
    tampered_thumb[7] ^= 0x80;
    chat.s3
        .put(1, &second_thumb.object_key(), tampered_thumb)
        .await
        .expect("a swapped thumb");
    for media in [second.id, second_thumb.id] {
        let (status, _, body) = fetch(&chat, posted.report, media).await;
        assert_eq!(status, StatusCode::GONE);
        assert_eq!(
            serde_json::from_slice::<Value>(&body).expect("json"),
            json!({"error": "chat_media_gone"})
        );
    }
    let mut forged = second.ciphertext.clone();
    forged.extend_from_slice(&[0u8; 4096]);
    chat.s3
        .put(1, &second.object_key(), forged)
        .await
        .expect("a longer object");
    assert_eq!(
        view_with(&chat, keys, "owner", posted.report, second.id).await,
        Err(ChatError::MediaGone)
    );

    chat.s3
        .delete(1, &first.object_key())
        .await
        .expect("an object removed behind the server");
    assert_eq!(
        view_with(&chat, keys, "owner", posted.report, first.id).await,
        Err(ChatError::MediaGone)
    );
    assert_eq!(views(&pool, posted.report).await.len(), served);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "needs SeaweedFS at CHAT_TEST_S3_ENDPOINT"]
async fn a_reported_backdrop_opens_from_the_room_meta(pool: PgPool) {
    let chat = bucketed(&pool).await;
    let keys = &chat.keks;
    let (_, report, backdrop) = room_backdrop(&pool, &chat).await;
    stored(&chat, &backdrop).await;
    set_status(&pool, backdrop.id, "held").await;
    assert_eq!(
        view_with(&chat, keys, "owner", report, backdrop.id).await,
        Ok(backdrop.plain.clone())
    );
    assert_eq!(
        views(&pool, report).await,
        vec![json!({"media": backdrop.id})]
    );
}
