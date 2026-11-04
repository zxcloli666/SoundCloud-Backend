use crate::modules::chat::model::{ConversationKind, MemberRole, MemberState};
use crate::modules::chat::{ChatError, ChatResult};

pub fn kind(raw: &str) -> ChatResult<ConversationKind> {
    match raw {
        "direct" => Ok(ConversationKind::Direct),
        "group" => Ok(ConversationKind::Group),
        "room" => Ok(ConversationKind::Room),
        "self" => Ok(ConversationKind::Saved),
        _ => Err(ChatError::Internal),
    }
}

pub fn role(raw: &str) -> ChatResult<MemberRole> {
    match raw {
        "owner" => Ok(MemberRole::Owner),
        "admin" => Ok(MemberRole::Admin),
        "member" => Ok(MemberRole::Member),
        _ => Err(ChatError::Internal),
    }
}

pub fn state(raw: &str) -> ChatResult<MemberState> {
    match raw {
        "active" => Ok(MemberState::Active),
        "invited" => Ok(MemberState::Invited),
        "left" => Ok(MemberState::Left),
        "kicked" => Ok(MemberState::Kicked),
        "banned" => Ok(MemberState::Banned),
        _ => Err(ChatError::Internal),
    }
}

pub fn role_name(role: MemberRole) -> &'static str {
    match role {
        MemberRole::Owner => "owner",
        MemberRole::Admin => "admin",
        MemberRole::Member => "member",
    }
}

pub fn state_name(state: MemberState) -> &'static str {
    match state {
        MemberState::Active => "active",
        MemberState::Invited => "invited",
        MemberState::Left => "left",
        MemberState::Kicked => "kicked",
        MemberState::Banned => "banned",
    }
}

pub fn is_admin(role: MemberRole) -> bool {
    matches!(role, MemberRole::Owner | MemberRole::Admin)
}

pub fn may_remove(actor: MemberRole, target: MemberRole) -> bool {
    match actor {
        MemberRole::Owner => target != MemberRole::Owner,
        MemberRole::Admin => target == MemberRole::Member,
        MemberRole::Member => false,
    }
}

pub fn may_set_ttl(kind: ConversationKind, role: MemberRole) -> ChatResult<()> {
    match kind {
        ConversationKind::Direct => Ok(()),
        ConversationKind::Group if is_admin(role) => Ok(()),
        ConversationKind::Group => Err(ChatError::NotMember),
        ConversationKind::Room | ConversationKind::Saved => Err(ChatError::Invalid),
    }
}

pub fn may_delete_for_everyone(kind: ConversationKind, role: MemberRole) -> ChatResult<()> {
    match kind {
        ConversationKind::Direct | ConversationKind::Saved => Ok(()),
        ConversationKind::Group if role == MemberRole::Owner => Ok(()),
        ConversationKind::Group => Err(ChatError::NotMember),
        ConversationKind::Room => Err(ChatError::Invalid),
    }
}

#[cfg(test)]
#[path = "../tests/member_roles_tests.rs"]
mod tests;
