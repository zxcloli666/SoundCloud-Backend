use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;

use crate::modules::chat::body::ChatJson;
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::media::limits::GET_TTL;
use crate::modules::chat::media::wire::{MediaUrl, UrlsAnswer, UrlsBody};
use crate::modules::chat::members::access;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub async fn media_urls(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatJson(body): ChatJson<UrlsBody>,
) -> ChatResult<Json<UrlsAnswer>> {
    limits::admit(&chat, Bucket::MediaUrls, &caller.user_id).await?;
    body.validate()?;
    sign_rows(&chat, &caller, &body).await.map(Json)
}

async fn sign_rows(
    chat: &ChatState,
    caller: &ChatCaller,
    body: &UrlsBody,
) -> ChatResult<UrlsAnswer> {
    let user = caller.user_id.as_str();
    let floor = match body.conversation_id {
        Some(conversation) => access::load_in(chat, conversation, user)
            .await?
            .readable()?
            .history_floor(),
        None => 0,
    };
    let rows = sqlx::query_file!(
        "queries/chat/media/urls_rows.sql",
        &body.ids,
        user,
        body.conversation_id,
        floor
    )
    .fetch_all(&chat.pg)
    .await?;
    let mut found: HashMap<_, _> = rows.into_iter().map(|row| (row.id, row)).collect();
    let mut items = Vec::with_capacity(body.ids.len());
    for id in &body.ids {
        let item = match found.remove(id) {
            Some(row) if row.own || row.shared => match row.status.as_str() {
                "uploaded" | "attached" => {
                    let store = u16::try_from(row.store).map_err(|_| ChatError::Internal)?;
                    let url = chat.s3.presign_get(store, &row.object_key, GET_TTL).await?;
                    MediaUrl::Ok { id: *id, url }
                }
                "lost" => MediaUrl::Lost { id: *id },
                _ => MediaUrl::Deleted { id: *id },
            },
            _ => MediaUrl::Deleted { id: *id },
        };
        items.push(item);
    }
    Ok(UrlsAnswer {
        expires_in_s: u32::try_from(GET_TTL.as_secs()).unwrap_or(u32::MAX),
        items,
    })
}

#[cfg(test)]
#[path = "../tests/media_urls_tests.rs"]
mod tests;
