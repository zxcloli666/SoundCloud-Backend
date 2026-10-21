use axum::extract::State;
use axum::http::StatusCode;
use sqlx::PgPool;

use super::{BLOCKS_MAX, block, blocked_between, blocked_by, list, unblock};
use crate::modules::chat::ChatError;
use crate::modules::chat::body::ChatPath;
use crate::modules::chat::moderation::testing::state;
use crate::modules::chat::test_support::caller;

#[sqlx::test(migrations = "./migrations")]
async fn a_block_works_both_ways_and_is_undone(pool: PgPool) {
    let chat = state(pool, None);
    assert_eq!(
        block(caller(), State(chat.clone()), ChatPath("202".to_owned()))
            .await
            .expect("blocked"),
        StatusCode::NO_CONTENT
    );
    block(caller(), State(chat.clone()), ChatPath("202".to_owned()))
        .await
        .expect("blocking twice is fine");
    assert!(blocked_between(&chat, "101", "202").await.expect("read"));
    assert!(blocked_between(&chat, "202", "101").await.expect("read"));
    assert!(blocked_by(&chat, "101", "202").await.expect("read"));
    assert!(!blocked_by(&chat, "202", "101").await.expect("read"));
    assert_eq!(list(&chat, "101", 10).await.expect("read").len(), 1);

    unblock(caller(), State(chat.clone()), ChatPath("202".to_owned()))
        .await
        .expect("unblocked");
    assert!(!blocked_between(&chat, "101", "202").await.expect("read"));
}

#[sqlx::test(migrations = "./migrations")]
async fn a_music_block_of_a_user_also_closes_chat(pool: PgPool) {
    let chat = state(pool.clone(), None);
    sqlx::query(
        "INSERT INTO user_blocked_artists (sc_user_id, kind, target_id, name, sc_user_ids)
         VALUES ('303', 'user', '404', 'x', ARRAY['404']), ('505', 'artist', '00000000-0000-0000-0000-000000000001', 'y', ARRAY['606'])",
    )
    .execute(&pool)
    .await
    .expect("music blocks");
    assert!(blocked_between(&chat, "404", "303").await.expect("read"));
    assert!(blocked_between(&chat, "303", "404").await.expect("read"));
    assert!(!blocked_between(&chat, "505", "606").await.expect("read"));
}

#[sqlx::test(migrations = "./migrations")]
async fn odd_targets_and_a_full_list_are_refused(pool: PgPool) {
    let chat = state(pool.clone(), None);
    assert_eq!(
        block(caller(), State(chat.clone()), ChatPath("101".to_owned())).await,
        Err(ChatError::Invalid)
    );
    assert_eq!(
        block(
            caller(),
            State(chat.clone()),
            ChatPath("not-a-user".to_owned())
        )
        .await,
        Err(ChatError::NotFound)
    );
    sqlx::query(
        "INSERT INTO chat.blocks (user_id, blocked_user) SELECT '101', g::text FROM generate_series(1, $1) g",
    )
    .bind(BLOCKS_MAX as i32)
    .execute(&pool)
    .await
    .expect("a full list");
    assert_eq!(
        block(caller(), State(chat.clone()), ChatPath("999999".to_owned())).await,
        Err(ChatError::RequestLimit)
    );
    block(caller(), State(chat), ChatPath("7".to_owned()))
        .await
        .expect("an existing block is still fine");
}
