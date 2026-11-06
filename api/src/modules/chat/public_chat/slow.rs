use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::json;

use crate::modules::chat::body::ChatJson;
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::public_chat::{self, ID, delivery, log, staff};
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub const SLOW_MODE_MAX_S: u32 = 600;

#[derive(Debug, Clone, Deserialize)]
pub struct SlowModeBody {
    pub seconds: u32,
}

pub async fn slow_mode(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatJson(body): ChatJson<SlowModeBody>,
) -> ChatResult<StatusCode> {
    public_chat::ensure_on(&chat)?;
    limits::admit(&chat, Bucket::PublicMod, &caller.user_id).await?;
    if body.seconds > SLOW_MODE_MAX_S {
        return Err(ChatError::Invalid);
    }
    staff::staff_role(&chat, &caller.user_id).await?;
    let mut tx = chat.pg.begin().await?;
    let seconds = i32::try_from(body.seconds).map_err(|_| ChatError::Invalid)?;
    let last_seq = sqlx::query_file_scalar!("queries/chat/public/slow_mode.sql", ID, seconds)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ChatError::NotFound)?;
    log::record_in(
        &mut tx,
        &caller.user_id,
        "slow_mode",
        &format!("conversation:{ID}"),
        json!({ "seconds": body.seconds }),
    )
    .await?;
    tx.commit().await?;
    delivery::announce(&chat, last_seq).await?;
    Ok(StatusCode::NO_CONTENT)
}
