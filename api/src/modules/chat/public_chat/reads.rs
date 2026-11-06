use crate::modules::chat::public_chat::ID;
use crate::modules::chat::sync::bump;
use crate::modules::chat::{ChatResult, ChatState};

pub async fn record(chat: &ChatState, user_id: &str, seq: i64) -> ChatResult<()> {
    let moved = sqlx::query_file_scalar!("queries/chat/public/read.sql", ID, user_id, seq)
        .fetch_optional(&chat.pg)
        .await?;
    if moved == Some(true) {
        bump::conversation(chat, ID, 0, &[user_id.to_owned()]).await?;
    }
    Ok(())
}
