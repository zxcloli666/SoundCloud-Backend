use axum::extract::State;
use chrono::Duration;
use sqlx::PgPool;
use uuid::Uuid;

use super::{list, session_is_young};
use crate::modules::chat::ChatCaller;
use crate::modules::chat::moderation::testing::{conversation, state};
use crate::modules::chat::test_support::caller;

async fn session(pool: &PgPool, age: Duration) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO sessions (id, created_at) VALUES ($1, (now() - $2::int * interval '1 second') AT TIME ZONE current_setting('TimeZone'))")
        .bind(id)
        .bind(age.num_seconds() as i32)
        .execute(pool)
        .await
        .expect("a session");
    id
}

#[sqlx::test(migrations = "./migrations")]
async fn a_session_younger_than_an_hour_is_young(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let young = session(&pool, Duration::minutes(10)).await;
    let old = session(&pool, Duration::hours(3)).await;
    assert!(session_is_young(&chat, young).await.expect("read"));
    assert!(!session_is_young(&chat, old).await.expect("read"));
    assert!(!session_is_young(&chat, Uuid::now_v7()).await.expect("read"));
}

#[sqlx::test(migrations = "./migrations")]
async fn requests_list_pending_chats_without_blocked_senders(pool: PgPool) {
    let chat = state(pool.clone(), None);
    let wanted = conversation(&pool, "direct", &[("7", true), ("101", false)]).await;
    conversation(&pool, "group", &[("8", true), ("101", false)]).await;
    conversation(&pool, "direct", &[("9", true), ("101", true)]).await;
    let blocked = conversation(&pool, "direct", &[("10", true), ("101", false)]).await;
    sqlx::query("INSERT INTO chat.blocks (user_id, blocked_user) VALUES ('101', '10')")
        .execute(&pool)
        .await
        .expect("a block");

    let shown = list(caller(), State(chat.clone()))
        .await
        .expect("requests")
        .0;
    assert_eq!(shown.count, 2);
    let ids: Vec<Uuid> = shown.requests.iter().map(|r| r.conversation_id).collect();
    assert!(ids.contains(&wanted));
    assert!(!ids.contains(&blocked));
    let from: Vec<&str> = shown.requests.iter().map(|r| r.from.as_str()).collect();
    assert!(from.contains(&"7") && from.contains(&"8"));

    let tight = ChatCaller {
        max_bytes: 300,
        ..caller()
    };
    let squeezed = list(tight, State(chat.clone())).await.expect("requests").0;
    assert_eq!((squeezed.count, squeezed.requests.len()), (2, 1));
}
