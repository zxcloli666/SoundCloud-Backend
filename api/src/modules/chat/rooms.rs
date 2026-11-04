pub mod bridge;
pub mod diff;
mod handlers;
pub mod lookup;
mod refusal;
pub mod shred;
pub mod suggest;
pub mod system;

use uuid::Uuid;

use crate::modules::chat::{ChatResult, ChatState};

pub use handlers::{chat, moderator, mute, preview, remove_member, settings, suggest};
pub use refusal::refusal;

pub async fn member_from_seq(
    chat: &ChatState,
    conversation: Uuid,
    user_id: &str,
) -> ChatResult<Option<i64>> {
    let member = lookup::member_of_chat(chat, conversation, user_id).await?;
    Ok(member.map(|member| member.chat_from_seq))
}

#[cfg(test)]
#[path = "tests/rooms_tests.rs"]
mod tests;
