use chrono::{Duration, Utc};
use sqlx::PgPool;

use super::{
    Restriction, Scope, card_for, check, lift, newcomer, restrict, restriction, valid_user_id,
};
use crate::modules::chat::ChatError;
use crate::modules::chat::moderation::testing::{aged_user, conversation, state};

#[test]
fn user_ids_are_sc_numbers_or_load_test_names() {
    for good in ["1", "123456789", "lt:42"] {
        assert!(valid_user_id(good), "{good}");
    }
    for bad in [
        "",
        "lt:",
        "abc",
        "12a",
        "soundcloud:users:1",
        "1 2",
        &"9".repeat(33),
    ] {
        assert!(!valid_user_id(bad), "{bad}");
    }
}

#[test]
fn a_ban_covers_every_scope_and_a_narrow_restriction_only_its_own() {
    let until = Utc::now();
    let ban = Restriction {
        until,
        scope: vec!["all".to_owned()],
    };
    assert!(ban.covers(Scope::DmNew) && ban.covers(Scope::Media) && ban.covers(Scope::All));
    let narrow = Restriction {
        until,
        scope: vec!["dm_new".to_owned(), "media".to_owned()],
    };
    assert!(narrow.covers(Scope::DmNew));
    assert!(!narrow.covers(Scope::All));
    assert_eq!(
        narrow.muted(),
        ChatError::Muted {
            until: until.timestamp_millis()
        }
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn restrictions_merge_and_never_shorten(pool: PgPool) {
    let chat = state(pool, None);
    let day = Utc::now() + Duration::hours(24);
    let week = Utc::now() + Duration::days(7);
    restrict(&chat, "9", week, &[Scope::DmNew])
        .await
        .expect("restricted");
    let merged = restrict(&chat, "9", day, &[Scope::Media, Scope::DmNew])
        .await
        .expect("restricted");
    assert_eq!(merged.scope, vec!["dm_new".to_owned(), "media".to_owned()]);
    assert!(merged.until > Utc::now() + Duration::days(6));
    assert!(check(&chat, "9", Scope::DmNew).await.is_err());
    assert!(check(&chat, "9", Scope::All).await.is_ok());

    sqlx::query(
        "UPDATE chat.users SET restricted_until = now() - interval '1 minute' WHERE user_id = '9'",
    )
    .execute(&chat.pg)
    .await
    .expect("an expired restriction");
    assert!(restriction(&chat, "9").await.expect("read").is_none());
    let fresh = restrict(&chat, "9", day, &[Scope::All])
        .await
        .expect("restricted");
    assert_eq!(fresh.scope, vec!["all".to_owned()]);
    assert!(matches!(
        check(&chat, "9", Scope::Media).await,
        Err(ChatError::Muted { .. })
    ));
    lift(&chat, "9").await.expect("lifted");
    assert!(check(&chat, "9", Scope::All).await.is_ok());
}

#[sqlx::test(migrations = "./migrations")]
async fn newcomers_are_users_first_seen_less_than_a_week_ago(pool: PgPool) {
    let chat = state(pool.clone(), None);
    assert!(newcomer(&chat, "5").await.expect("read"));
    aged_user(&pool, "5", 2).await;
    assert!(newcomer(&chat, "5").await.expect("read"));
    aged_user(&pool, "5", 9).await;
    assert!(!newcomer(&chat, "5").await.expect("read"));
}

#[sqlx::test(migrations = "./migrations")]
async fn a_card_shows_the_profile_and_how_the_viewer_can_write(pool: PgPool) {
    let chat = state(pool.clone(), None);
    sqlx::query(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized, avatar_url, verified)
         VALUES ('62', 'soundcloud:users:62', 'Neo', 'neo', 'https://a/x.jpg', true)",
    )
    .execute(&pool)
    .await
    .expect("a profile");

    let stranger = card_for(&chat, "61", "62").await.expect("a card");
    assert_eq!(stranger.username.as_deref(), Some("Neo"));
    assert!(stranger.verified);
    assert!(!stranger.contact && !stranger.blocked);
    assert_eq!(stranger.can_message, "unavailable");
    assert_eq!(stranger.presence, None);

    sqlx::query("INSERT INTO chat.users (user_id, dm_policy) VALUES ('62', 'everyone')")
        .execute(&pool)
        .await
        .expect("open dms");
    assert_eq!(
        card_for(&chat, "61", "62")
            .await
            .expect("a card")
            .can_message,
        "request"
    );

    conversation(&pool, "direct", &[("61", true), ("62", true)]).await;
    let contact = card_for(&chat, "61", "62").await.expect("a card");
    assert!(contact.contact);
    assert_eq!(contact.can_message, "direct");

    sqlx::query("INSERT INTO chat.blocks (user_id, blocked_user) VALUES ('61', '62')")
        .execute(&pool)
        .await
        .expect("a block");
    let blocked = card_for(&chat, "61", "62").await.expect("a card");
    assert!(blocked.blocked);
    assert_eq!(blocked.can_message, "unavailable");
    let seen_by_blocked = card_for(&chat, "62", "61").await.expect("a card");
    assert!(!seen_by_blocked.blocked);
    assert_eq!(seen_by_blocked.can_message, "unavailable");
    assert_eq!(seen_by_blocked.username, None);
}
