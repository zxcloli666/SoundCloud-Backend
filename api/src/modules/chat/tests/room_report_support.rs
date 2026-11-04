use axum::http::StatusCode;
use serde_json::{Value, json};
use uuid::Uuid;

use super::backdrop_support::Bench;
use crate::modules::chat::b64;
use crate::modules::chat::devices::test_support::{call, caller, enroll};

pub fn report(conversation: Uuid, media: Uuid) -> Value {
    json!({"conversation_id": conversation, "kind": "backdrop", "media_id": media, "reason": "harassment", "comment": " ugly "})
}

pub async fn file(bench: &Bench, user: &str, body: Value) -> anyhow::Result<(StatusCode, Value)> {
    bench.ask(user, "POST", "/chat/reports", Some(body)).await
}

pub struct Stage {
    pub code: String,
    pub conversation: Uuid,
    pub media: Uuid,
    pub meta: Vec<u8>,
    pub meta_epoch: i64,
}

pub async fn stage(
    bench: &Bench,
    host: &str,
    public: bool,
    guests: &[&str],
) -> anyhow::Result<Stage> {
    bench.star(host).await?;
    let code = bench.room(host, public, guests).await?;
    let conversation = bench.open_chat(&code, host).await?;
    let media = bench
        .upload(host, conversation, "backdrop", "uploaded")
        .await?;
    let (status, view) = bench
        .ask(
            host,
            "PUT",
            &format!("/chat/rooms/{code}/settings"),
            Some(json!({"backdrop": {"kind": "image", "mediaId": media}})),
        )
        .await?;
    assert_eq!(status, StatusCode::OK, "{view}");
    let device = enroll(&bench.chat, &bench.pg, host).await;
    let who = caller(host, device.session, Some(&device.row));
    let (status, keys) = call(
        &bench.chat,
        who.clone(),
        "GET",
        &format!("/chat/conversations/{conversation}/keys"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{keys}");
    let meta_epoch = keys[0]["epoch"].as_i64().expect("an epoch");
    let meta = vec![3_u8; 300];
    let (status, sent) = call(
        &bench.chat,
        who,
        "POST",
        &format!("/chat/conversations/{conversation}/messages"),
        Some(json!({"client_id": Uuid::now_v7(), "class": "meta", "epoch": meta_epoch, "body": b64::encode(&meta), "media": []})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    Ok(Stage {
        code,
        conversation,
        media,
        meta,
        meta_epoch,
    })
}

pub async fn holds(bench: &Bench, media: Uuid) -> anyhow::Result<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT report_id FROM chat.media_holds WHERE media_id = $1 AND until > now() + interval '29 days' ORDER BY report_id",
    )
    .bind(media)
    .fetch_all(&bench.pg)
    .await?)
}
