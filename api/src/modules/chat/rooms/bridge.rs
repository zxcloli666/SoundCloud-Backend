use tracing::warn;
use uuid::Uuid;

use crate::modules::chat::rooms::diff::{departed, rows};
use crate::modules::chat::rooms::{fanout, system};
use crate::modules::chat::{ChatResult, ChatState};
use crate::modules::rooms::model::Room;
use crate::modules::rooms::service::{Cause, OpenedChat};

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub async fn after_change(chat: &ChatState, before: &Room, after: &Room, cause: Cause<'_>) {
    let Some(conversation) = after.chat_id.or(before.chat_id) else {
        return;
    };
    let rows = rows(before, after, cause, now_ms());
    let departed = departed(before, after);
    if rows.is_empty() && departed.is_empty() {
        return;
    }
    let mut users: Vec<String> = after.members.iter().map(|m| m.user_id.clone()).collect();
    let rekey = !departed.is_empty();
    users.extend(departed);
    let outcome = async {
        let seq = system::post(chat, conversation, &rows, rekey).await?;
        match seq {
            Some(seq) => fanout::bump(chat, conversation, seq, &users).await,
            None => Ok(()),
        }
    };
    if let Err(error) = outcome.await {
        warn!(%conversation, ?error, "room chat did not follow a room change");
    }
}

pub async fn ensure_conversation(chat: &ChatState, opened: &OpenedChat) -> ChatResult<()> {
    sqlx::query_file!(
        "queries/chat/rooms/insert_conversation.sql",
        opened.conversation_id,
        opened.room.code,
        opened.room.host_id
    )
    .execute(&chat.pg)
    .await?;
    Ok(())
}

pub async fn last_seq(chat: &ChatState, conversation: Uuid) -> ChatResult<i64> {
    let seq = sqlx::query_file_scalar!("queries/chat/rooms/last_seq.sql", conversation)
        .fetch_optional(&chat.pg)
        .await?;
    Ok(seq.unwrap_or(0))
}

pub async fn keep_alive(chat: &ChatState, conversation: Uuid) {
    let touched = sqlx::query_file!("queries/chat/rooms/keep_alive.sql", conversation)
        .execute(&chat.pg)
        .await;
    if let Err(error) = touched {
        warn!(%conversation, %error, "room chat liveness was not extended");
    }
}
