use chrono::{Duration, Utc};

use super::*;
use crate::modules::chat::members::access::GroupSettings;
use crate::modules::chat::model::MemberRole;

fn input(class: &str, target_seq: Option<i64>, body: usize) -> SendMessage {
    SendMessage {
        client_id: Uuid::from_u128(9),
        class: class.to_owned(),
        epoch: 0,
        target_seq,
        body: vec![7; body],
    }
}

fn access(kind: ConversationKind, role: MemberRole) -> Access {
    Access {
        conversation: Uuid::from_u128(1),
        user_id: "101".to_owned(),
        kind,
        peer: None,
        key_epoch: 0,
        last_seq: 0,
        ttl_s: None,
        settings: GroupSettings::default(),
        role,
        state: MemberState::Active,
        accepted: true,
        joined_seq: 0,
        hidden_before_seq: 0,
        peer_state: None,
        recipients: vec!["101".to_owned()],
        public_room: false,
    }
}

fn target(sender: &str, age_hours: i64) -> Target {
    Target {
        sender_user: sender.to_owned(),
        class: "post".to_owned(),
        created_at: Utc::now() - Duration::hours(age_hours),
        expires_at: None,
        deleted: false,
    }
}

#[test]
fn a_well_formed_post_passes() {
    let out = outgoing(input("post", None, 300)).expect("a post");
    assert_eq!(out.class, Class::Post);
    assert_eq!(out.target_seq, None);
}

#[test]
fn clients_can_never_write_system_rows() {
    assert_eq!(outgoing(input("system", None, 10)), Err(ChatError::Invalid));
    assert_eq!(outgoing(input("shout", None, 10)), Err(ChatError::Invalid));
}

#[test]
fn bodies_are_never_empty_or_over_48_kb() {
    assert_eq!(outgoing(input("post", None, 0)), Err(ChatError::Invalid));
    assert_eq!(
        outgoing(input("post", None, BODY_MAX + 1)),
        Err(ChatError::Invalid)
    );
    assert!(outgoing(input("post", None, BODY_MAX)).is_ok());
    assert!(outgoing(input("meta", None, META_MAX)).is_ok());
    assert_eq!(
        outgoing(input("meta", None, META_MAX + 1)),
        Err(ChatError::Invalid)
    );
}

#[test]
fn edits_reactions_and_pins_need_a_target_and_posts_refuse_one() {
    for class in ["edit", "react", "pin"] {
        assert_eq!(outgoing(input(class, None, 10)), Err(ChatError::Invalid));
        assert!(outgoing(input(class, Some(3), 10)).is_ok());
        assert_eq!(outgoing(input(class, Some(0), 10)), Err(ChatError::Invalid));
    }
    for class in ["post", "meta"] {
        assert_eq!(outgoing(input(class, Some(3), 10)), Err(ChatError::Invalid));
    }
}

#[test]
fn meta_is_for_group_admins_only() {
    let group_member = access(ConversationKind::Group, MemberRole::Member);
    let group_admin = access(ConversationKind::Group, MemberRole::Admin);
    let dm = access(ConversationKind::Direct, MemberRole::Member);
    assert_eq!(
        may_post(&group_member, Class::Meta),
        Err(ChatError::NotMember)
    );
    assert_eq!(may_post(&group_admin, Class::Meta), Ok(()));
    assert_eq!(may_post(&dm, Class::Meta), Err(ChatError::Invalid));
}

#[test]
fn pins_need_a_group_admin_but_not_in_a_dm() {
    let group_member = access(ConversationKind::Group, MemberRole::Member);
    let dm = access(ConversationKind::Direct, MemberRole::Member);
    assert_eq!(
        may_post(&group_member, Class::Pin),
        Err(ChatError::NotMember)
    );
    assert_eq!(may_post(&dm, Class::Pin), Ok(()));
    assert_eq!(may_post(&group_member, Class::React), Ok(()));
}

#[test]
fn room_moderators_pin_and_guests_read_from_their_seat() {
    let guest = Access {
        joined_seq: 7,
        ..access(ConversationKind::Room, MemberRole::Member)
    };
    let moderator = access(ConversationKind::Room, MemberRole::Admin);
    assert_eq!(may_post(&guest, Class::Pin), Err(ChatError::NotMember));
    assert_eq!(may_post(&moderator, Class::Pin), Ok(()));
    assert_eq!(may_post(&guest, Class::Meta), Err(ChatError::Invalid));
    assert_eq!(guest.history_floor(), 7);
    assert!(moderator.moderates() && !guest.moderates());
}

#[test]
fn an_admins_only_group_silences_members_but_lets_them_react() {
    let mut member = access(ConversationKind::Group, MemberRole::Member);
    member.settings.admins_only = true;
    assert_eq!(may_post(&member, Class::Post), Err(ChatError::NotMember));
    assert_eq!(may_post(&member, Class::React), Ok(()));
    let mut owner = access(ConversationKind::Group, MemberRole::Owner);
    owner.settings.admins_only = true;
    assert_eq!(may_post(&owner, Class::Post), Ok(()));
}

#[test]
fn an_invited_member_cannot_post_before_accepting() {
    let mut invited = access(ConversationKind::Direct, MemberRole::Member);
    invited.state = MemberState::Invited;
    assert_eq!(may_post(&invited, Class::Post), Err(ChatError::NotMember));
}

#[test]
fn only_the_author_edits_and_only_within_48_hours() {
    let now = Utc::now();
    assert_eq!(
        inherited_expiry(Class::Edit, "101", Some(&target("101", 47)), now),
        Ok(None)
    );
    assert_eq!(
        inherited_expiry(Class::Edit, "101", Some(&target("101", 49)), now),
        Err(ChatError::Invalid)
    );
    assert_eq!(
        inherited_expiry(Class::Edit, "101", Some(&target("202", 1)), now),
        Err(ChatError::NotMember)
    );
    assert_eq!(
        inherited_expiry(Class::React, "101", Some(&target("202", 100)), now),
        Ok(None)
    );
}

#[test]
fn a_missing_deleted_or_non_post_target_is_refused() {
    let now = Utc::now();
    assert_eq!(
        inherited_expiry(Class::React, "101", None, now),
        Err(ChatError::NotFound)
    );
    let mut deleted = target("202", 1);
    deleted.deleted = true;
    assert_eq!(
        inherited_expiry(Class::React, "101", Some(&deleted), now),
        Err(ChatError::NotFound)
    );
    let mut system = target("202", 1);
    system.class = "system".to_owned();
    assert_eq!(
        inherited_expiry(Class::Pin, "101", Some(&system), now),
        Err(ChatError::Invalid)
    );
}

#[test]
fn edits_reactions_and_pins_inherit_the_expiry_and_die_with_the_target() {
    let now = Utc::now();
    let mut expiring = target("101", 1);
    let at = now + Duration::minutes(5);
    expiring.expires_at = Some(at);
    for class in [Class::Edit, Class::React, Class::Pin] {
        assert_eq!(
            inherited_expiry(class, "101", Some(&expiring), now),
            Ok(Some(at))
        );
    }
    expiring.expires_at = Some(now - Duration::seconds(1));
    for class in [Class::Edit, Class::React, Class::Pin] {
        assert_eq!(
            inherited_expiry(class, "101", Some(&expiring), now),
            Err(ChatError::NotFound)
        );
    }
}

#[test]
fn only_posts_take_the_chat_timer() {
    let at = Utc::now();
    assert_eq!(
        expiry(Class::Post, Some(3_600), None, at),
        Some(at + Duration::seconds(3_600))
    );
    assert_eq!(expiry(Class::Post, None, None, at), None);
    let inherited = Some(at + Duration::minutes(1));
    assert_eq!(expiry(Class::React, Some(3_600), inherited, at), inherited);
    assert_eq!(expiry(Class::React, Some(3_600), None, at), None);
    assert_eq!(expiry(Class::Meta, Some(3_600), None, at), None);
}
