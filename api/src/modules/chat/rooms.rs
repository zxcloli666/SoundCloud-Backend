pub mod bridge;
pub mod conversation;
pub mod diff;
mod handlers;
pub mod lookup;
pub mod media_gate;
pub mod perks;
mod refusal;
pub mod settings;
pub mod shred;
pub mod suggest;
pub mod system;

use uuid::Uuid;

use crate::modules::chat::{ChatResult, ChatState};

pub use handlers::{chat, moderator, mute, preview, remove_member, suggest};
pub use refusal::refusal;
pub use settings::settings;

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

#[cfg(test)]
#[path = "tests/room_backdrop_support.rs"]
mod backdrop_support;

#[cfg(test)]
#[path = "tests/room_backdrop_tests.rs"]
mod backdrop_tests;

#[cfg(test)]
#[path = "tests/room_meta_tests.rs"]
mod meta_tests;

#[cfg(test)]
#[path = "tests/room_backdrop_report_tests.rs"]
mod report_tests;

#[cfg(test)]
#[path = "tests/room_backdrop_race_tests.rs"]
mod race_tests;

#[cfg(test)]
#[path = "tests/room_media_gate_tests.rs"]
mod gate_tests;

#[cfg(test)]
#[path = "tests/room_report_support.rs"]
mod report_support;

#[cfg(test)]
#[path = "tests/room_backdrop_recovery_tests.rs"]
mod recovery_tests;
