use uuid::Uuid;

use super::{may_set, needs_star};
use crate::modules::chat::ChatError;
use crate::modules::chat::members::access::{Access, GroupSettings};
use crate::modules::chat::model::{ConversationKind, MemberRole, MemberState};
use crate::modules::rooms::model::Backdrop;

fn access(kind: ConversationKind, role: MemberRole, peer_state: Option<MemberState>) -> Access {
    Access {
        conversation: Uuid::from_u128(1),
        user_id: "101".to_owned(),
        kind,
        peer: None,
        last_seq: 0,
        settings: GroupSettings::default(),
        backdrop: None,
        role,
        state: MemberState::Active,
        accepted: true,
        joined_seq: 0,
        hidden_before_seq: 0,
        peer_state,
        recipients: vec!["101".to_owned()],
        public_room: false,
    }
}

#[test]
fn members_of_a_direct_chat_admins_of_a_group_staff_of_the_public_chat_and_the_saved_owner_set_it()
{
    let open = access(
        ConversationKind::Direct,
        MemberRole::Member,
        Some(MemberState::Active),
    );
    assert_eq!(may_set(&open), Ok(()));
    let pending = access(
        ConversationKind::Direct,
        MemberRole::Member,
        Some(MemberState::Invited),
    );
    assert_eq!(may_set(&pending), Err(ChatError::Unavailable));
    let mut unanswered = open.clone();
    unanswered.accepted = false;
    assert_eq!(may_set(&unanswered), Ok(()));
    assert_eq!(
        may_set(&access(ConversationKind::Saved, MemberRole::Member, None)),
        Ok(())
    );
    for kind in [ConversationKind::Group, ConversationKind::Public] {
        assert_eq!(may_set(&access(kind, MemberRole::Owner, None)), Ok(()));
        assert_eq!(may_set(&access(kind, MemberRole::Admin, None)), Ok(()));
        assert_eq!(
            may_set(&access(kind, MemberRole::Member, None)),
            Err(ChatError::NotMember)
        );
    }
    assert_eq!(
        may_set(&access(ConversationKind::Room, MemberRole::Owner, None)),
        Err(ChatError::Invalid)
    );
}

#[test]
fn only_a_public_preset_needs_no_star() {
    let preset = Backdrop::Preset {
        id: "aurora".to_owned(),
    };
    let image = Backdrop::Image {
        media_id: Uuid::from_u128(7),
    };
    assert!(!needs_star(ConversationKind::Public, &preset));
    assert!(needs_star(ConversationKind::Public, &image));
    for kind in [
        ConversationKind::Direct,
        ConversationKind::Group,
        ConversationKind::Saved,
    ] {
        assert!(needs_star(kind, &preset));
        assert!(needs_star(kind, &image));
    }
}
