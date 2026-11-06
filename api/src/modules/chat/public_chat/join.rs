use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::public_chat::view::{self, PublicView};
use crate::modules::chat::public_chat::{self, ID};
use crate::modules::chat::{ChatCaller, ChatResult, ChatState};

pub async fn open(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
) -> ChatResult<Json<PublicView>> {
    public_chat::ensure_on(&chat)?;
    limits::admit(&chat, Bucket::ReadList, &caller.user_id).await?;
    public_chat::gate::read(&chat, &caller.user_id).await?;
    let mut row = view::row(&chat, &caller.user_id).await?;
    if !row.joined() {
        sqlx::query_file!("queries/chat/public/join.sql", ID, caller.user_id)
            .execute(&chat.pg)
            .await?;
        row = view::row(&chat, &caller.user_id).await?;
    }
    Ok(Json(view::opened(&chat, row).await?))
}
