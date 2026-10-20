use tracing::warn;
use uuid::Uuid;

use crate::modules::chat::rooms::fanout;
use crate::modules::chat::{ChatResult, ChatState};
use crate::modules::rooms::model::Room;

pub async fn end_chat(chat: &ChatState, room: &Room) {
    let Some(conversation) = room.chat_id else {
        return;
    };
    let members: Vec<String> = room.members.iter().map(|m| m.user_id.clone()).collect();
    if let Err(error) = shred(chat, conversation, &members).await {
        warn!(%conversation, ?error, "an ended room kept its chat");
    }
}

pub async fn shred(chat: &ChatState, conversation: Uuid, members: &[String]) -> ChatResult<bool> {
    let mut tx = chat.pg.begin().await?;
    let shredded = sqlx::query_file!("queries/chat/rooms/shred.sql", conversation)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        == 1;
    if shredded {
        sqlx::query_file!("queries/chat/rooms/drop_keys.sql", conversation)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    chat.hot.forget(conversation);
    fanout::bump(chat, conversation, 0, members).await?;
    Ok(shredded)
}
