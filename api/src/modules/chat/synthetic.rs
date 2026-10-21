mod layout;
mod purge;
mod seed;
mod sessions;

use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use serde::Deserialize;
use serde_json::json;

use crate::modules::chat::admin::audit;
use crate::modules::chat::admin::auth::ChatAdmin;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub use purge::Purged;
pub use seed::Seeded;
pub use sessions::session;

pub const USER_PREFIX: &str = "lt:";

const AUDIT_TARGET: &str = "loadtest";

#[derive(Debug, Clone, Deserialize)]
pub struct Seed {
    pub users: u32,
    pub devices_per_user: u32,
    pub groups: u32,
}

pub fn is_synthetic(user_id: &str) -> bool {
    user_id.starts_with(USER_PREFIX)
}

pub async fn seed(
    admin: ChatAdmin,
    State(chat): State<Arc<ChatState>>,
    body: Bytes,
) -> ChatResult<Json<Seeded>> {
    if !chat.config.loadtest {
        return Err(ChatError::NotFound);
    }
    let input: Seed = serde_json::from_slice(&body).map_err(|_| ChatError::Invalid)?;
    let seeded = seed::run(&chat, &input).await?;
    let detail = json!({
        "first": seeded.first,
        "users": input.users,
        "devices": seeded.devices.len(),
        "conversations": seeded.conversations.len(),
    });
    audit::record(&chat, &admin.admin, "loadtest_seed", AUDIT_TARGET, detail).await?;
    Ok(Json(seeded))
}

pub async fn purge(
    admin: ChatAdmin,
    State(chat): State<Arc<ChatState>>,
) -> ChatResult<Json<Purged>> {
    if !chat.config.loadtest {
        return Err(ChatError::NotFound);
    }
    let purged = purge::run(&chat).await?;
    let detail = serde_json::to_value(&purged).map_err(|_| ChatError::Internal)?;
    audit::record(&chat, &admin.admin, "loadtest_purge", AUDIT_TARGET, detail).await?;
    Ok(Json(purged))
}

#[cfg(test)]
#[path = "tests/synthetic_tests/mod.rs"]
mod tests;
