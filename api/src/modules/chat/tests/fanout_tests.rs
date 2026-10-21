use std::sync::Arc;

use chrono::Utc;
use redis::AsyncCommands;
use uuid::Uuid;

use super::*;
use crate::modules::chat::ChatState;
use crate::modules::chat::rt_keys::{self, DELTAS_KEPT};
use crate::modules::chat::sweeps::hooks;
use crate::modules::chat::test_support::chat_state;

fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned())
}

fn state() -> Arc<ChatState> {
    let pg = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://scd@127.0.0.1:1/none")
        .expect("a lazy pool");
    let redis = crate::redis::connect_url(&redis_url()).expect("a redis pool");
    chat_state(pg, redis, true)
}

fn users() -> Vec<String> {
    let run = Uuid::now_v7().simple().to_string();
    vec![format!("fan-{run}-a"), format!("fan-{run}-b")]
}

fn row(conversation: Uuid, seq: i64) -> MessageRow {
    MessageRow {
        conversation_id: conversation,
        seq,
        client_id: Uuid::now_v7(),
        sender_user: "101".to_owned(),
        sender_device: None,
        class: "post".to_owned(),
        epoch: 0,
        target_seq: None,
        body: Some(vec![1; 32]),
        created_at: Utc::now(),
        expires_at: None,
    }
}

async fn score_of(chat: &ChatState, user: &str, conversation: Uuid) -> Option<f64> {
    let mut conn = chat.redis.get().await.expect("a redis connection");
    conn.zscore(rt_keys::deltas(user), conversation.to_string())
        .await
        .expect("a score")
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn a_bump_moves_every_recipient_and_never_lowers_the_seq() -> anyhow::Result<()> {
    let chat = state();
    let conversation = Uuid::now_v7();
    let users = users();
    bump(&chat, conversation, 5, &users).await.expect("a bump");
    let first: Vec<Option<f64>> =
        futures::future::join_all(users.iter().map(|user| score_of(&chat, user, conversation)))
            .await;
    bump(&chat, conversation, 3, &users).await.expect("a bump");
    let mut conn = chat.redis.get().await?;
    let seq: i64 = conn.get(rt_keys::conversation_seq(conversation)).await?;
    assert_eq!(seq, 5);
    for (user, before) in users.iter().zip(first) {
        let before = before.expect("the first bump left a delta");
        let after = score_of(&chat, user, conversation)
            .await
            .expect("the second bump left a delta");
        assert!(after > before, "{user}: {after} after {before}");
        let ttl: i64 = conn.ttl(rt_keys::deltas(user)).await?;
        assert!(ttl > 0);
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn a_send_bump_and_a_sweep_bump_share_one_version_layout() -> anyhow::Result<()> {
    let chat = state();
    let conversation = Uuid::now_v7();
    let users = users();
    bump(&chat, conversation, 1, &users)
        .await
        .expect("a send bump");
    let mut conn = chat.redis.get().await?;
    let first: i64 = conn.hget(rt_keys::version(&users[0]), "v").await?;
    drop(conn);
    hooks::messages_gone(&chat, conversation, &[1], &users)
        .await
        .expect("a sweep bump");
    bump(&chat, conversation, 2, &users)
        .await
        .expect("a send bump");
    let mut conn = chat.redis.get().await?;
    let last: i64 = conn.hget(rt_keys::version(&users[0]), "v").await?;
    assert!(last > first, "{last} after {first}");
    let seq: i64 = conn.get(rt_keys::conversation_seq(conversation)).await?;
    assert_eq!(seq, 2);
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn a_request_change_reaches_every_named_user() -> anyhow::Result<()> {
    let chat = state();
    let users = users();
    requests(&chat, &users).await.expect("a requests bump");
    let mut conn = chat.redis.get().await?;
    for user in &users {
        let score: Option<f64> = conn.zscore(rt_keys::deltas(user), "requests").await?;
        assert!(score.is_some(), "{user}");
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn the_delta_set_keeps_only_the_newest_conversations() -> anyhow::Result<()> {
    let chat = state();
    let users = users();
    let total = usize::try_from(DELTAS_KEPT).expect("small") + 20;
    let mut newest = Uuid::nil();
    for _ in 0..total {
        newest = Uuid::now_v7();
        bump(&chat, newest, 1, &users[..1]).await.expect("a bump");
    }
    let mut conn = chat.redis.get().await?;
    let kept: i64 = conn.zcard(rt_keys::deltas(&users[0])).await?;
    assert_eq!(kept, i64::try_from(DELTAS_KEPT)?);
    assert!(score_of(&chat, &users[0], newest).await.is_some());
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn publish_fills_the_hot_cache_before_the_bump() -> anyhow::Result<()> {
    let chat = state();
    let conversation = Uuid::now_v7();
    let rows = vec![row(conversation, 1), row(conversation, 2)];
    publish(&chat, &rows, &users()).await.expect("a publish");
    let cached = chat.hot.after(conversation, 0).expect("a cached tail");
    assert_eq!(
        cached.iter().map(|row| row.seq).collect::<Vec<_>>(),
        vec![1, 2]
    );
    let mut conn = chat.redis.get().await?;
    let seq: i64 = conn.get(rt_keys::conversation_seq(conversation)).await?;
    assert_eq!(seq, 2);
    Ok(())
}

#[tokio::test]
async fn no_rows_touch_no_redis() {
    let chat = crate::modules::chat::test_support::offline_chat_state(true);
    assert!(publish(&chat, &[], &users()).await.is_ok());
}

#[tokio::test]
async fn a_dead_redis_is_busy_not_internal() {
    let chat = crate::modules::chat::test_support::offline_chat_state(true);
    let answer = bump(&chat, Uuid::now_v7(), 1, &users()).await;
    assert!(matches!(
        answer,
        Err(crate::modules::chat::ChatError::Busy { .. })
    ));
}
