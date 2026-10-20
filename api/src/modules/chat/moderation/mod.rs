pub mod client;
pub mod verdict;

use uuid::Uuid;

use crate::modules::chat::moderation::client::AiClient;
use crate::modules::chat::moderation::verdict::Verdict;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const AUDIT_ADMIN: &str = "ai-moderator";
pub const CALLS_IN_FLIGHT: usize = 4;
pub const CALL_TIMEOUT_SECS: u64 = 30;

pub struct Moderator {
    client: AiClient,
}

impl Moderator {
    pub fn new(client: AiClient) -> Self {
        Self { client }
    }

    pub fn ai_enabled(&self) -> bool {
        self.client.enabled()
    }

    pub async fn judge(&self, _chat: &ChatState, _report: Uuid) -> ChatResult<Option<Verdict>> {
        Err(ChatError::Unimplemented)
    }
}
