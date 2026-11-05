use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::backdrop_support::{Bench, someone};
use super::meta_tests::{keys, meta};
use crate::modules::chat::devices::test_support::enroll;
use crate::modules::chat::{ChatState, rt_keys};

async fn forget_sends(chat: &Arc<ChatState>, user: &str, conversation: Uuid) -> anyhow::Result<()> {
    let mut conn = chat.redis.get().await?;
    let _: i64 = redis::cmd("DEL")
        .arg(rt_keys::limit("send:20s", user))
        .arg(rt_keys::limit("send:1m", user))
        .arg(rt_keys::limit(&format!("send:{conversation}:5s"), user))
        .query_async(&mut conn)
        .await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_looping_host_runs_out_of_room_meta(pg: PgPool) -> anyhow::Result<()> {
    let bench = Bench::new(pg.clone())?;
    let chat = bench.chat.clone();
    let host = someone();
    bench.star(&host).await?;
    let code = bench.room(&host, false, &[]).await?;
    let conversation = bench.open_chat(&code, &host).await?;
    let device = enroll(&chat, &pg, &host).await;
    let epoch = *keys(&chat, &host, &device, conversation)
        .await
        .last()
        .expect("a key");
    for round in 0..12 {
        forget_sends(&chat, &host, conversation).await?;
        let (status, sent) = meta(&chat, &host, &device, conversation, epoch, &[round]).await;
        assert_eq!(status, StatusCode::OK, "{sent}");
    }
    forget_sends(&chat, &host, conversation).await?;
    assert_eq!(
        meta(&chat, &host, &device, conversation, epoch, b"again").await,
        (
            StatusCode::TOO_MANY_REQUESTS,
            json!({"error": "chat_send_limited"})
        )
    );
    chat.rooms.end(&code).await?;
    Ok(())
}
