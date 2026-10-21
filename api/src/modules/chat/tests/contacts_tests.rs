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

#[sqlx::test(migrations = "./migrations")]
async fn only_accepted_direct_or_group_chats_and_follows_make_a_contact(pool: sqlx::PgPool) {
    use crate::modules::chat::moderation::testing::conversation;
    conversation(&pool, "direct", &[("1", true), ("2", false)]).await;
    conversation(&pool, "room", &[("1", true), ("2", true)]).await;
    assert_eq!(relation(&pool, "1", "2").await.expect("read"), STRANGER);
    conversation(&pool, "group", &[("1", true), ("2", true)]).await;
    assert!(relation(&pool, "1", "2").await.expect("read").contact);

    sqlx::query(
        "INSERT INTO user_followings (user_id, target_user_urn) VALUES ('soundcloud:users:4', 'soundcloud:users:3')",
    )
    .execute(&pool)
    .await
    .expect("a follow");
    assert!(relation(&pool, "3", "4").await.expect("read").follows);
    assert!(!relation(&pool, "4", "3").await.expect("read").follows);
}

#[sqlx::test(migrations = "./migrations")]
async fn a_block_either_way_closes_the_dm_and_self_is_always_open(pool: sqlx::PgPool) {
    let chat = crate::modules::chat::moderation::testing::state(pool.clone(), None);
    sqlx::query("INSERT INTO chat.users (user_id, dm_policy) VALUES ('2', 'everyone')")
        .execute(&pool)
        .await
        .expect("open dms");
    assert_eq!(
        landing(&chat, "1", "2", STRANGER).await.expect("read"),
        Some(MemberState::Invited)
    );
    sqlx::query("INSERT INTO chat.blocks (user_id, blocked_user) VALUES ('2', '1')")
        .execute(&pool)
        .await
        .expect("a block");
    assert_eq!(
        landing(&chat, "1", "2", STRANGER).await.expect("read"),
        None
    );
    assert_eq!(landing(&chat, "2", "1", CONTACT).await.expect("read"), None);
    assert_eq!(
        landing(&chat, "1", "1", STRANGER).await.expect("read"),
        Some(MemberState::Active)
    );
}
