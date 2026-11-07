use crate::modules::chat::members::access::Access;
use crate::modules::chat::model::{ConversationKind, MemberState};
use crate::modules::chat::{ChatError, ChatResult};
use crate::modules::rooms::model::Backdrop;

pub fn may_set(access: &Access) -> ChatResult<()> {
    match access.kind {
        ConversationKind::Direct if access.peer_state == Some(MemberState::Active) => Ok(()),
        ConversationKind::Direct => Err(ChatError::Unavailable),
        ConversationKind::Saved => Ok(()),
        ConversationKind::Group | ConversationKind::Public if access.is_admin() => Ok(()),
        ConversationKind::Group | ConversationKind::Public => Err(ChatError::NotMember),
        ConversationKind::Room => Err(ChatError::Invalid),
    }
}

pub fn needs_star(kind: ConversationKind, backdrop: &Backdrop) -> bool {
    !(kind == ConversationKind::Public && matches!(backdrop, Backdrop::Preset { .. }))
}

#[cfg(test)]
#[path = "../tests/chat_backdrop_rules_tests.rs"]
mod tests;
