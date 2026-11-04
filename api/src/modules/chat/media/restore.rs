use std::collections::HashSet;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use uuid::Uuid;

use crate::modules::chat::body::ChatJson;
use crate::modules::chat::limits::{self, Bucket};
use crate::modules::chat::media::wire::{Picked, RestoreAnswer, Selection};
use crate::modules::chat::quota;
use crate::modules::chat::{ChatCaller, ChatResult, ChatState};

pub async fn restore_media(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
    ChatJson(selection): ChatJson<Selection>,
) -> ChatResult<Json<RestoreAnswer>> {
    limits::admit(&chat, Bucket::MediaRestore, &caller.user_id).await?;
    let picked = selection.pick()?;
    restore_rows(&chat, &caller, &picked).await.map(Json)
}

async fn restore_rows(
    chat: &ChatState,
    caller: &ChatCaller,
    picked: &Picked,
) -> ChatResult<RestoreAnswer> {
    let user = caller.user_id.as_str();
    let restored = quota::restore(&chat.pg, user, picked).await?;
    let asked = match picked {
        Picked::Ids(ids) => roots_of(chat, user, ids).await?,
        Picked::All => restored.roots,
    };
    Ok(RestoreAnswer {
        restored: restored.roots,
        missed: asked.saturating_sub(restored.roots),
        used: restored.used.total(),
        quota: quota::quota(),
    })
}

async fn roots_of(chat: &ChatState, user: &str, ids: &[Uuid]) -> ChatResult<u32> {
    let parents = sqlx::query_file!("queries/chat/media/roots_of.sql", ids, user)
        .fetch_all(&chat.pg)
        .await?;
    let mut roots: HashSet<Uuid> = ids.iter().copied().collect();
    for row in parents {
        roots.remove(&row.id);
        roots.insert(row.root);
    }
    Ok(u32::try_from(roots.len()).unwrap_or(u32::MAX))
}
