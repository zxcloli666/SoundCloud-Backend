pub mod delete;
pub mod delivery;
pub mod gate;
pub mod hidden;
mod join;
pub mod log;
pub mod pings;
pub mod reads;
pub mod sanctions;
mod slow;
pub mod staff;
pub mod view;
pub mod watchers;

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use uuid::Uuid;

use crate::modules::chat::{ChatCaller, ChatError, ChatResult, ChatState};

pub use join::open;
pub use log::moderation_log;
pub use sanctions::{ban, mute, unban, unmute};
pub use slow::slow_mode;
pub use staff::moderator;

pub const ID: Uuid = Uuid::from_u128(0x00000000_0000_7000_8000_7075626c6963);
pub const RETENTION_DAYS: i64 = 7;

pub fn ensure_on(chat: &ChatState) -> ChatResult<()> {
    if chat.config.public {
        Ok(())
    } else {
        Err(ChatError::Disabled)
    }
}

pub fn guard(chat: &ChatState, conversation: Uuid) -> ChatResult<()> {
    if conversation == ID {
        ensure_on(chat)
    } else {
        Ok(())
    }
}

pub async fn watch(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
) -> ChatResult<StatusCode> {
    ensure_on(&chat)?;
    crate::modules::chat::limits::admit(
        &chat,
        crate::modules::chat::limits::Bucket::PublicWatch,
        &caller.user_id,
    )
    .await?;
    watchers::touch(&chat, &caller.user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unwatch(
    caller: ChatCaller,
    State(chat): State<Arc<ChatState>>,
) -> ChatResult<StatusCode> {
    ensure_on(&chat)?;
    watchers::remove(&chat, &caller.user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
#[path = "tests/public_support.rs"]
pub mod support;

#[cfg(test)]
#[path = "tests/public_join_tests.rs"]
mod join_tests;

#[cfg(test)]
#[path = "tests/public_send_tests.rs"]
mod send_tests;

#[cfg(test)]
#[path = "tests/public_staff_tests.rs"]
mod staff_tests;

#[cfg(test)]
#[path = "tests/public_delivery_tests.rs"]
mod delivery_tests;

#[cfg(test)]
#[path = "tests/public_upkeep_tests.rs"]
mod upkeep_tests;

#[cfg(test)]
#[path = "tests/public_ping_tests.rs"]
mod ping_tests;

#[cfg(test)]
#[path = "tests/public_cap_tests.rs"]
mod cap_tests;
