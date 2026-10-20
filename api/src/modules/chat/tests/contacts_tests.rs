use super::*;

const STRANGER: Relation = Relation {
    follows: false,
    contact: false,
};
const FOLLOWED_BY: Relation = Relation {
    follows: true,
    contact: false,
};
const CONTACT: Relation = Relation {
    follows: false,
    contact: true,
};

#[test]
fn a_contact_always_gets_in() {
    for policy in [DmPolicy::Everyone, DmPolicy::Following, DmPolicy::Nobody] {
        assert_eq!(placement(CONTACT, policy), Some(MemberState::Active));
    }
}

#[test]
fn someone_the_recipient_follows_gets_in_unless_they_said_nobody() {
    assert_eq!(
        placement(FOLLOWED_BY, DmPolicy::Everyone),
        Some(MemberState::Active)
    );
    assert_eq!(
        placement(FOLLOWED_BY, DmPolicy::Following),
        Some(MemberState::Active)
    );
    assert_eq!(placement(FOLLOWED_BY, DmPolicy::Nobody), None);
}

#[test]
fn a_stranger_lands_in_requests_only_when_everyone_may_write() {
    assert_eq!(
        placement(STRANGER, DmPolicy::Everyone),
        Some(MemberState::Invited)
    );
    assert_eq!(placement(STRANGER, DmPolicy::Following), None);
    assert_eq!(placement(STRANGER, DmPolicy::Nobody), None);
}

#[test]
fn an_unknown_policy_reads_as_the_default() {
    assert_eq!(DmPolicy::parse("following"), DmPolicy::Following);
    assert_eq!(DmPolicy::parse("everyone"), DmPolicy::Everyone);
    assert_eq!(DmPolicy::parse("nobody"), DmPolicy::Nobody);
    assert_eq!(DmPolicy::parse("strange"), DmPolicy::Following);
}
