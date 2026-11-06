use std::sync::Arc;

use serde_json::json;

use crate::modules::chat::members::access::Access;
use crate::modules::chat::model::MessageRow;
use crate::modules::chat::public_chat::{delivery, log};
use crate::modules::chat::{ChatResult, ChatState, admin::audit};

pub async fn after(
    chat: &Arc<ChatState>,
    access: &Access,
    seq: i64,
    rows: &[MessageRow],
) -> ChatResult<()> {
    if rows.is_empty() {
        return delivery::announce(chat, access.last_seq).await;
    }
    let taken = rows
        .iter()
        .find(|row| row.seq == seq && row.sender_user != access.user_id);
    if let Some(row) = taken {
        log::record(
            chat,
            &access.user_id,
            "delete",
            &audit::message(access.conversation, seq),
            json!({ "author": row.sender_user, "seq": seq }),
        )
        .await?;
    }
    delivery::publish(chat, rows).await
}
