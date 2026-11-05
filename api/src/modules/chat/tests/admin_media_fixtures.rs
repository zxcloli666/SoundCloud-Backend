use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use sc_chat_wire::msg::Class;
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use super::cipher::Role;
use super::media_support::{
    Row, Upload, backdrop_report, conversation_key, hold, image, media_row, meta_envelope,
    post_envelope, send, single, webp,
};
use super::resolve::{AdminDecision, Resolve};
use super::router;
use crate::modules::chat::ChatState;
use crate::modules::chat::admin::audit;
use crate::modules::chat::media::MediaConfig;
use crate::modules::chat::moderation::testing::{ADMIN_TOKEN, audit_actions, conversation, post};
use crate::modules::chat::reports::{FileReport, file_with};
use crate::modules::chat::s3::S3;

pub struct Posted {
    pub group: Uuid,
    pub report: Uuid,
    pub meta_seq: i64,
    pub post_seq: i64,
    pub avatar: Upload,
    pub images: Vec<(Upload, Upload)>,
}

pub async fn group_report(pool: &PgPool, chat: &ChatState) -> Posted {
    let keys = &chat.keks;
    let group = conversation(pool, "group", &[("91", true), ("92", true)]).await;
    let ck = conversation_key(pool, keys, group).await;
    post(pool, group, &ck, "91", "look at this").await;
    let avatar = Upload::new(Role::Image, &webp("avatar", 900));
    let meta = send(
        pool,
        group,
        &ck,
        "92",
        Class::Meta,
        &meta_envelope(Some(single(&avatar, 512, 512))),
    )
    .await;
    let images = vec![
        (
            Upload::new(Role::Image, &webp("first", 6_000)),
            Upload::new(Role::Thumb, &webp("first-thumb", 700)),
        ),
        (
            Upload::new(Role::Image, &webp("second", 9_000)),
            Upload::new(Role::Thumb, &webp("second-thumb", 800)),
        ),
    ];
    let attachments = images
        .iter()
        .map(|(full, thumb)| image(full, thumb))
        .collect();
    let posted = send(
        pool,
        group,
        &ck,
        "92",
        Class::Post,
        &post_envelope("bad pictures", attachments),
    )
    .await;
    for (full, thumb) in &images {
        attached(pool, full, Some(thumb), "92", group).await;
    }
    attached(pool, &avatar, None, "92", group).await;
    let report = file_with(
        chat,
        keys,
        "91",
        &FileReport {
            conversation_id: group,
            seq_from: meta.seq,
            seq_to: posted.seq,
            reason: "other".to_owned(),
            comment: None,
        },
    )
    .await
    .expect("a report")
    .id;
    Posted {
        group,
        report,
        meta_seq: meta.seq,
        post_seq: posted.seq,
        avatar,
        images,
    }
}

pub async fn attached(
    pool: &PgPool,
    root: &Upload,
    thumb: Option<&Upload>,
    owner: &str,
    origin: Uuid,
) {
    let kind = if thumb.is_some() { "image" } else { "avatar" };
    media_row(
        pool,
        Row {
            upload: root,
            owner,
            origin,
            kind,
            parent: None,
            status: "attached",
        },
    )
    .await;
    if let Some(thumb) = thumb {
        media_row(
            pool,
            Row {
                upload: thumb,
                owner,
                origin,
                kind: "thumb",
                parent: Some(root.id),
                status: "attached",
            },
        )
        .await;
    }
}

pub async fn set_status(pool: &PgPool, root: Uuid, status: &str) {
    sqlx::query(
        "UPDATE chat.media
         SET status = $2,
             delete_after = CASE WHEN $2 = 'uploaded' THEN now() + interval '1 hour' END
         WHERE id = $1 OR parent_id = $1",
    )
    .bind(root)
    .bind(status)
    .execute(pool)
    .await
    .expect("a status");
}

pub fn decision(decision: AdminDecision) -> Resolve {
    Resolve {
        decision,
        delete_seqs: Vec::new(),
        ban_days: None,
        ban_permanent: false,
        lift_restriction: false,
        clear_backdrop: false,
    }
}

pub async fn get(chat: &Arc<ChatState>, uri: &str) -> (StatusCode, Value) {
    let request = Request::get(uri)
        .header("x-chat-admin-token", ADMIN_TOKEN)
        .header("x-chat-admin-user", "owner")
        .body(Body::empty())
        .expect("a request");
    let app: axum::Router = router(chat.clone());
    let response = app.oneshot(request).await.expect("an answer");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("a body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

pub async fn view_rows(pool: &PgPool, report: Uuid) -> Vec<Value> {
    audit_actions(pool, &audit::report(report))
        .await
        .into_iter()
        .filter(|row| row.1 == "view_media")
        .map(|row| row.2)
        .collect()
}

pub fn with_store(
    chat: Arc<ChatState>,
    endpoint: &str,
    access: &str,
    secret: &str,
) -> Arc<ChatState> {
    with_bucket(chat, endpoint, access, secret, "scd-chat-test")
}

pub fn with_bucket(
    chat: Arc<ChatState>,
    endpoint: &str,
    access: &str,
    secret: &str,
    bucket: &str,
) -> Arc<ChatState> {
    let dir = std::env::temp_dir().join(format!("chat-admin-s3-{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("a keys dir");
    std::fs::write(dir.join("s3-secret-1"), secret).expect("a secret");
    let pairs = [
        ("CHAT_STORES", "1".to_owned()),
        ("CHAT_STORE_WRITE", "1".to_owned()),
        ("CHAT_STORE_1_ENDPOINT", endpoint.to_owned()),
        ("CHAT_STORE_1_PRESIGN", endpoint.to_owned()),
        ("CHAT_STORE_1_BUCKET", bucket.to_owned()),
        ("CHAT_STORE_1_ACCESS_KEY", access.to_owned()),
    ];
    let config = MediaConfig::parse(
        |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
        },
        &dir,
    )
    .expect("a media config");
    std::fs::remove_dir_all(&dir).expect("the keys dir goes");
    let mut chat = Arc::into_inner(chat).expect("a fresh state");
    chat.s3 = S3::new(&config);
    chat.config.media = config;
    Arc::new(chat)
}

pub async fn room_backdrop(pool: &PgPool, chat: &ChatState) -> (Uuid, Uuid, Upload) {
    let keys = &chat.keks;
    let room = conversation(pool, "room", &[("61", true), ("62", true)]).await;
    let ck = conversation_key(pool, keys, room).await;
    let backdrop = Upload::new(Role::Image, &webp("backdrop", 20_000));
    media_row(
        pool,
        Row {
            upload: &backdrop,
            owner: "61",
            origin: room,
            kind: "backdrop",
            parent: None,
            status: "attached",
        },
    )
    .await;
    let meta = send(
        pool,
        room,
        &ck,
        "61",
        Class::Meta,
        &meta_envelope(Some(single(&backdrop, 1920, 1080))),
    )
    .await;
    let report = backdrop_report(chat, room, "61", "62", &meta, backdrop.id).await;
    hold(pool, backdrop.id, report).await;
    (room, report, backdrop)
}
