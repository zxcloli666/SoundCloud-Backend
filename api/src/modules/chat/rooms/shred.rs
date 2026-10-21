use tracing::warn;
use uuid::Uuid;

use crate::modules::chat::rooms::fanout;
use crate::modules::chat::sweeps::purge;
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
    let shredded = purge::shred(chat, conversation).await?;
    fanout::bump(chat, conversation, 0, members).await?;
    Ok(shredded)
}
