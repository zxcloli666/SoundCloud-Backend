use crate::modules::chat::{ChatError, ChatResult, ChatState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bucket {
    Send,
    Create,
    Request,
    Invite,
    Report,
    Device,
    ReadMessages,
    ReadList,
    ReadKeys,
    ReadCard,
    Preview,
}

impl Bucket {
    pub fn name(self) -> &'static str {
        match self {
            Bucket::Send => "send",
            Bucket::Create => "create",
            Bucket::Request => "request",
            Bucket::Invite => "invite",
            Bucket::Report => "report",
            Bucket::Device => "device",
            Bucket::ReadMessages => "read_messages",
            Bucket::ReadList => "read_list",
            Bucket::ReadKeys => "read_keys",
            Bucket::ReadCard => "read_card",
            Bucket::Preview => "preview",
        }
    }
}

pub async fn admit(_chat: &ChatState, _bucket: Bucket, _user_id: &str) -> ChatResult<()> {
    Err(ChatError::Unimplemented)
}
