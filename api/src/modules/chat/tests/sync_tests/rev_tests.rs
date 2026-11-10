use sqlx::PgPool;
use uuid::Uuid;

use super::test_helpers::{conversation, messages, user, with_postgres};
use crate::modules::chat::ChatState;
use crate::modules::chat::sync::seats;

async fn rev(chat: &ChatState, user_id: &str, id: Uuid) -> i64 {
    seats::load(chat, user_id, &[id])
        .await
        .expect("seats")
        .get(&id)
        .expect("a seat")
        .rev
}

async fn run(pool: &PgPool, statement: &str, id: Uuid, user_id: &str) {
    sqlx::query(statement)
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await
        .expect("the row changes");
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_view_rev_moves_with_the_view_and_stays_under_messages_and_reads(pool: PgPool) {
    let chat = with_postgres(&pool);
    let (alice, bob) = (user(), user());
    let id = conversation(&pool, "direct", &[&alice, &bob]).await;
    let first = rev(&chat, &alice, id).await;
    assert_eq!(rev(&chat, &bob, id).await, first);

    messages(&pool, id, &bob, 3, 16).await;
    run(
        &pool,
        "UPDATE chat.members SET last_read_seq = 3 WHERE conversation_id = $1 AND user_id = $2",
        id,
        &alice,
    )
    .await;
    assert_eq!(rev(&chat, &alice, id).await, first);

    run(
        &pool,
        "UPDATE chat.conversations SET settings = settings || jsonb_build_object('backdrop', jsonb_build_object('kind', 'preset', 'id', 'aurora', 'by', $2::text)) WHERE id = $1",
        id,
        &alice,
    )
    .await;
    let dressed = rev(&chat, &alice, id).await;
    assert_ne!(dressed, first);
    assert_eq!(rev(&chat, &bob, id).await, dressed);

    run(
        &pool,
        "UPDATE chat.members SET muted_until = now() + interval '1 hour' WHERE conversation_id = $1 AND user_id = $2",
        id,
        &alice,
    )
    .await;
    let muted = rev(&chat, &alice, id).await;
    assert_ne!(muted, dressed);
    assert_eq!(rev(&chat, &bob, id).await, dressed);

    run(
        &pool,
        "UPDATE chat.members SET last_ping_seq = 4 WHERE conversation_id = $1 AND user_id = $2",
        id,
        &alice,
    )
    .await;
    let pinged = rev(&chat, &alice, id).await;
    assert_ne!(pinged, muted);
    run(
        &pool,
        "UPDATE chat.members SET last_read_seq = 4 WHERE conversation_id = $1 AND user_id = $2",
        id,
        &alice,
    )
    .await;
    assert_eq!(rev(&chat, &alice, id).await, muted);
}
