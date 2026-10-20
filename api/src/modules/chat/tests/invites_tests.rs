use axum::extract::State;
use chrono::{Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::{CreateInvite, create_with, join_with, parse_token, revoke};
use crate::modules::chat::ChatError;
use crate::modules::chat::body::ChatPath;
use crate::modules::chat::moderation::testing::{conversation, state};
use crate::modules::chat::test_support::caller;
use crate::modules::chat::users::{self, Scope};

async fn member(pool: &PgPool, conversation: Uuid, user: &str) -> Option<(String, bool, i64)> {
    sqlx::query_as(
        "SELECT state, accepted, joined_seq FROM chat.members WHERE conversation_id = $1 AND user_id = $2",
    )
    .bind(conversation)
    .bind(user)
    .fetch_optional(pool)
    .await
    .expect("read")
}

async fn member_count(pool: &PgPool, conversation: Uuid) -> i32 {
    sqlx::query_scalar("SELECT member_count FROM chat.conversations WHERE id = $1")
        .bind(conversation)
        .fetch_one(pool)
        .await
        .expect("read")
}

#[test]
fn a_token_is_32_random_bytes_in_url_safe_base64() {
    assert!(parse_token("short").is_none());
    assert!(parse_token("not base64 at all!").is_none());
    assert!(parse_token(&"A".repeat(43)).is_some());
}

#[sqlx::test(migrations = "./migrations")]
async fn only_group_admins_make_links(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let group = conversation(&pool, "group", &[("101", true), ("2", true)]).await;
    let dm = conversation(&pool, "direct", &[("101", true), ("3", true)]).await;
    let none = CreateInvite::default();

    let link = create_with(&chat, "101", group, &none)
        .await
        .expect("a link");
    assert_eq!(link.token.len(), 43);
    assert!(link.expires_at > (Utc::now() + Duration::days(6)).timestamp_millis());
    assert_eq!(link.uses_left, None);
    assert_eq!(
        create_with(&chat, "2", group, &none).await.err(),
        Some(ChatError::Invalid)
    );
    assert_eq!(
        create_with(&chat, "9", group, &none).await.err(),
        Some(ChatError::NotMember)
    );
    assert_eq!(
        create_with(&chat, "101", dm, &none).await.err(),
        Some(ChatError::Invalid)
    );
    for odd in [
        CreateInvite {
            expires_in_s: Some(10),
            uses: None,
        },
        CreateInvite {
            expires_in_s: None,
            uses: Some(0),
        },
    ] {
        assert_eq!(
            create_with(&chat, "101", group, &odd).await.err(),
            Some(ChatError::Invalid)
        );
    }
    users::restrict(&chat, "101", Utc::now() + Duration::hours(1), &[Scope::All])
        .await
        .expect("banned");
    assert!(matches!(
        create_with(&chat, "101", group, &none).await,
        Err(ChatError::Muted { .. })
    ));
}

#[sqlx::test(migrations = "./migrations")]
async fn a_link_adds_an_accepted_member_and_runs_out_of_uses(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let group = conversation(&pool, "group", &[("101", true), ("2", true)]).await;
    sqlx::query("UPDATE chat.conversations SET last_seq = 40, key_epoch = 3 WHERE id = $1")
        .bind(group)
        .execute(&pool)
        .await
        .expect("history");
    let link = create_with(
        &chat,
        "101",
        group,
        &CreateInvite {
            expires_in_s: None,
            uses: Some(1),
        },
    )
    .await
    .expect("a link");

    let joined = join_with(&chat, "5", &link.token).await.expect("joined");
    assert_eq!((joined.conversation_id, joined.joined), (group, true));
    assert_eq!(
        member(&pool, group, "5").await,
        Some(("active".to_owned(), true, 40))
    );
    assert_eq!(member_count(&pool, group).await, 3);
    let again = join_with(&chat, "5", &link.token)
        .await
        .expect("already in");
    assert!(!again.joined);
    assert_eq!(
        join_with(&chat, "6", &link.token).await.err(),
        Some(ChatError::NotFound)
    );
    assert_eq!(
        join_with(&chat, "6", "garbage").await.err(),
        Some(ChatError::NotFound)
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn bans_blocks_full_groups_and_revoked_links_keep_people_out(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let group = conversation(&pool, "group", &[("101", true), ("2", true)]).await;
    let link = create_with(&chat, "101", group, &CreateInvite::default())
        .await
        .expect("a link");

    sqlx::query(
        "INSERT INTO chat.members (conversation_id, user_id, state) VALUES ($1, '7', 'banned')",
    )
    .bind(group)
    .execute(&pool)
    .await
    .expect("a ban");
    assert_eq!(
        join_with(&chat, "7", &link.token).await.err(),
        Some(ChatError::Unavailable)
    );

    sqlx::query("INSERT INTO chat.blocks (user_id, blocked_user) VALUES ('8', '101')")
        .execute(&pool)
        .await
        .expect("a block");
    assert_eq!(
        join_with(&chat, "8", &link.token).await.err(),
        Some(ChatError::Unavailable)
    );

    users::restrict(&chat, "9", Utc::now() + Duration::hours(1), &[Scope::All])
        .await
        .expect("banned");
    assert!(matches!(
        join_with(&chat, "9", &link.token).await,
        Err(ChatError::Muted { .. })
    ));

    sqlx::query("UPDATE chat.conversations SET member_count = 200 WHERE id = $1")
        .bind(group)
        .execute(&pool)
        .await
        .expect("a full group");
    assert_eq!(
        join_with(&chat, "10", &link.token).await.err(),
        Some(ChatError::Unavailable)
    );
    assert_eq!(member(&pool, group, "10").await, None);
    sqlx::query("UPDATE chat.conversations SET member_count = 2 WHERE id = $1")
        .bind(group)
        .execute(&pool)
        .await
        .expect("room again");

    let mut stranger = caller();
    stranger.user_id = "2".to_owned();
    assert_eq!(
        revoke(stranger, State(chat.clone()), ChatPath(link.token.clone())).await,
        Err(ChatError::NotFound)
    );
    revoke(caller(), State(chat.clone()), ChatPath(link.token.clone()))
        .await
        .expect("revoked");
    assert_eq!(
        join_with(&chat, "11", &link.token).await.err(),
        Some(ChatError::NotFound)
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn an_expired_link_or_a_deleted_group_is_gone(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let group = conversation(&pool, "group", &[("101", true)]).await;
    let link = create_with(&chat, "101", group, &CreateInvite::default())
        .await
        .expect("a link");
    sqlx::query("UPDATE chat.invites SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .expect("expired");
    assert_eq!(
        join_with(&chat, "5", &link.token).await.err(),
        Some(ChatError::NotFound)
    );

    let other = create_with(&chat, "101", group, &CreateInvite::default())
        .await
        .expect("a link");
    sqlx::query("UPDATE chat.conversations SET deleted_at = now() WHERE id = $1")
        .bind(group)
        .execute(&pool)
        .await
        .expect("deleted");
    assert_eq!(
        join_with(&chat, "5", &other.token).await.err(),
        Some(ChatError::NotFound)
    );
}
