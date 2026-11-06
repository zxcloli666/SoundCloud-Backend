use std::sync::{Arc, LazyLock};
use std::time::Duration;

use redis::Script;
use tokio::time::sleep;

use crate::modules::chat::model::MessageRow;
use crate::modules::chat::public_chat::{ID, watchers};
use crate::modules::chat::sync::bump;
use crate::modules::chat::{ChatResult, ChatState, rt_keys};

static WINDOW: LazyLock<Script> =
    LazyLock::new(|| Script::new(include_str!("../lua/public_window.lua")));

pub const WAKE_EVERY: Duration = Duration::from_secs(2);
const TRAIL_SLACK: Duration = Duration::from_millis(25);
const WAKES: &str = "chat_public_wakes_total";

enum Window {
    Now,
    Later(Duration),
    Covered,
}

pub async fn publish(chat: &Arc<ChatState>, rows: &[MessageRow]) -> ChatResult<()> {
    let Some(last) = rows.iter().map(|row| row.seq).max() else {
        return Ok(());
    };
    for row in rows {
        chat.hot.put(row.clone());
    }
    announce(chat, last).await
}

pub async fn announce(chat: &Arc<ChatState>, seq: i64) -> ChatResult<()> {
    match window(chat).await? {
        Window::Now => wake(chat, seq).await,
        Window::Later(after) => {
            trail(chat.clone(), after);
            Ok(())
        }
        Window::Covered => Ok(()),
    }
}

async fn window(chat: &ChatState) -> ChatResult<Window> {
    let mut conn = chat.redis.get().await?;
    let code: i64 = WINDOW
        .key(rt_keys::PUBLIC_GATE)
        .key(rt_keys::PUBLIC_TRAIL)
        .arg(WAKE_EVERY.as_millis() as u64)
        .invoke_async(&mut conn)
        .await?;
    Ok(match code {
        0 => Window::Now,
        left if left > 0 => Window::Later(Duration::from_millis((left - 1) as u64)),
        _ => Window::Covered,
    })
}

fn trail(chat: Arc<ChatState>, after: Duration) {
    tokio::spawn(async move {
        tokio::select! {
            () = chat.shutdown.cancelled() => return,
            () = sleep(after + TRAIL_SLACK) => {}
        }
        if let Err(error) = flush(&chat).await {
            tracing::warn!(code = error.code(), "a public chat wake was not sent");
        }
    });
}

pub async fn flush(chat: &ChatState) -> ChatResult<()> {
    let mut conn = chat.redis.get().await?;
    let taken: Option<String> = redis::cmd("SET")
        .arg(rt_keys::PUBLIC_GATE)
        .arg(1)
        .arg("NX")
        .arg("PX")
        .arg(WAKE_EVERY.as_millis() as u64)
        .query_async(&mut conn)
        .await?;
    drop(conn);
    if taken.is_none() {
        return Ok(());
    }
    let seq = sqlx::query_file_scalar!("queries/chat/public/last_seq.sql", ID)
        .fetch_optional(&chat.pg)
        .await?
        .unwrap_or(0);
    wake(chat, seq).await
}

async fn wake(chat: &ChatState, seq: i64) -> ChatResult<()> {
    let users = watchers::current(chat).await?;
    bump::conversation(chat, ID, seq, &users).await?;
    metrics::counter!(WAKES).increment(1);
    Ok(())
}
