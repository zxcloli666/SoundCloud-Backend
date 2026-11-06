use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use futures::{StreamExt, TryStreamExt, stream};

use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::media::limits::{QUOTA_BYTES, USAGE_CONVERSATIONS_MAX};
use crate::modules::chat::media::wire::{ConversationUsage, UsageAnswer};
use crate::modules::chat::public_chat;
use crate::modules::chat::quota::unsigned;
use crate::modules::chat::rooms::lookup;
use crate::modules::chat::{ChatCaller, ChatResult, ChatState};

const ROOM_LOOKUPS: usize = 8;

pub async fn media_usage(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
) -> ChatResult<Json<UsageAnswer>> {
    limits::admit(&chat, Bucket::MediaUsage, &caller.user_id).await?;
    usage_rows(&chat, &caller).await.map(Json)
}

async fn usage_rows(chat: &ChatState, caller: &ChatCaller) -> ChatResult<UsageAnswer> {
    let user = caller.user_id.as_str();
    let totals = sqlx::query_file!("queries/chat/media/usage_full.sql", user)
        .fetch_optional(&chat.pg)
        .await?;
    let rows = sqlx::query_file!(
        "queries/chat/media/usage_conversations.sql",
        user,
        USAGE_CONVERSATIONS_MAX as i64
    )
    .fetch_all(&chat.pg)
    .await?;
    let sees_public = caller.sees_public();
    let conversations = stream::iter(rows)
        .filter(|row| std::future::ready(sees_public || row.conversation_id != public_chat::ID))
        .map(|row| async move {
            let gone = match (row.live, row.kind.as_deref()) {
                (false, _) | (_, None) => true,
                (true, Some("room")) => lookup::member_of_chat(chat, row.conversation_id, user)
                    .await?
                    .is_none(),
                (true, Some(_)) => !row.member,
            };
            ChatResult::Ok(ConversationUsage {
                conversation_id: row.conversation_id,
                bytes: unsigned(row.bytes),
                files: u32::try_from(row.files).unwrap_or(0),
                gone,
            })
        })
        .buffered(ROOM_LOOKUPS)
        .try_collect()
        .await?;
    let (bytes, reserved, files) =
        totals.map_or((0, 0, 0), |row| (row.bytes, row.reserved, row.files));
    Ok(UsageAnswer {
        bytes: unsigned(bytes),
        reserved: unsigned(reserved),
        quota: QUOTA_BYTES,
        files: u32::try_from(files).unwrap_or(0),
        conversations,
    })
}
