use super::*;
use crate::modules::chat::ChatError;
use crate::modules::chat::model::{ConversationKind, MemberRole, MemberState};

const ROLES: [MemberRole; 3] = [MemberRole::Owner, MemberRole::Admin, MemberRole::Member];
const STATES: [MemberState; 5] = [
    MemberState::Active,
    MemberState::Invited,
    MemberState::Left,
    MemberState::Kicked,
    MemberState::Banned,
];

#[test]
fn names_round_trip_through_the_database_spelling() {
    for one in ROLES {
        assert_eq!(role(role_name(one)), Ok(one));
    }
    for one in STATES {
        assert_eq!(state(state_name(one)), Ok(one));
    }
    for one in [
        ConversationKind::Direct,
        ConversationKind::Group,
        ConversationKind::Room,
        ConversationKind::Saved,
    ] {
        assert_eq!(kind(one.as_str()), Ok(one));
    }
    assert_eq!(role("root"), Err(ChatError::Internal));
}

#[test]
fn only_a_higher_role_removes_a_member() {
    use MemberRole::{Admin, Member, Owner};
    assert!(may_remove(Owner, Admin));
    assert!(may_remove(Owner, Member));
    assert!(!may_remove(Owner, Owner));
    assert!(may_remove(Admin, Member));
    assert!(!may_remove(Admin, Admin));
    assert!(!may_remove(Admin, Owner));
    for target in ROLES {
        assert!(!may_remove(Member, target));
    }
}

#[test]
fn any_dm_member_sets_the_timer_but_only_group_admins_do() {
    for one in ROLES {
        assert_eq!(may_set_ttl(ConversationKind::Direct, one), Ok(()));
    }
    assert_eq!(
        may_set_ttl(ConversationKind::Group, MemberRole::Owner),
        Ok(())
    );
    assert_eq!(
        may_set_ttl(ConversationKind::Group, MemberRole::Admin),
        Ok(())
    );
    assert_eq!(
        may_set_ttl(ConversationKind::Group, MemberRole::Member),
        Err(ChatError::NotMember)
    );
}

#[test]
fn rooms_and_saved_have_no_timer() {
    for one in ROLES {
        assert_eq!(
            may_set_ttl(ConversationKind::Room, one),
            Err(ChatError::Invalid)
        );
        assert_eq!(
            may_set_ttl(ConversationKind::Saved, one),
            Err(ChatError::Invalid)
        );
    }
}

#[test]
fn only_the_group_owner_deletes_a_group_for_everyone() {
    assert_eq!(
        may_delete_for_everyone(ConversationKind::Group, MemberRole::Owner),
        Ok(())
    );
    assert_eq!(
        may_delete_for_everyone(ConversationKind::Group, MemberRole::Admin),
        Err(ChatError::NotMember)
    );
    assert_eq!(
        may_delete_for_everyone(ConversationKind::Direct, MemberRole::Member),
        Ok(())
    );
    assert_eq!(
        may_delete_for_everyone(ConversationKind::Room, MemberRole::Owner),
        Err(ChatError::Invalid)
    );
}
