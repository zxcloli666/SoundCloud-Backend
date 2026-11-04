use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

use super::session_entry;
use crate::modules::chat::ChatError;
use crate::modules::chat::test_support::chat_state;

const SUBSCRIPTION: &str = "chat_resolve_stalled_feed";

async fn stall_the_feed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(&format!(
        "CREATE SUBSCRIPTION {SUBSCRIPTION}
         CONNECTION 'host=127.0.0.1 port=1 dbname=none' PUBLICATION none
         WITH (connect = false, slot_name = '{SUBSCRIPTION}', create_slot = false, enabled = false);
         ALTER SUBSCRIPTION {SUBSCRIPTION} ENABLE;"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

async fn heal_the_feed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(&format!(
        "ALTER SUBSCRIPTION {SUBSCRIPTION} DISABLE;
         ALTER SUBSCRIPTION {SUBSCRIPTION} SET (slot_name = NONE);
         DROP SUBSCRIPTION {SUBSCRIPTION};"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_session_miss_during_a_replication_stall_asks_for_a_retry_not_a_login(
    pool: PgPool,
) -> anyhow::Result<()> {
    let redis = crate::redis::connect_url("redis://127.0.0.1:1")?;
    let chat = chat_state(pool.clone(), redis, true);
    let missing = Uuid::now_v7();

    let local = session_entry(&chat, missing).await;
    assert!(matches!(local, Ok(None)), "{local:?}");

    stall_the_feed(&pool).await?;
    let missing = Uuid::now_v7();
    let stalled = session_entry(&chat, missing).await;
    heal_the_feed(&pool).await?;

    let error = stalled.expect_err("a stalled feed must not answer a session");
    assert_eq!(error, ChatError::SessionNotReplicated);
    assert_eq!(error.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error.body()["error"], "chat_session_not_replicated");
    assert_eq!(error.body()["retry_ms"], 5_000);
    Ok(())
}

async fn stored_session(pool: &PgPool, user: &str) -> anyhow::Result<Uuid> {
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, $2, $2, 'access', 'refresh', now() + interval '1 hour', '')",
    )
    .bind(connection)
    .bind(user)
    .execute(pool)
    .await?;
    let session = Uuid::now_v7();
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(session)
        .bind(connection)
        .execute(pool)
        .await?;
    Ok(session)
}

#[sqlx::test(migrations = "./migrations")]
async fn a_flood_of_random_session_ids_does_not_starve_a_real_user_and_unknown_ids_are_remembered(
    pool: PgPool,
) -> anyhow::Result<()> {
    let redis = crate::redis::connect_url("redis://127.0.0.1:1")?;
    let chat = chat_state(pool.clone(), redis, true);
    let real = stored_session(&pool, "4242").await?;

    let mut flood = tokio::task::JoinSet::new();
    for _ in 0..48 {
        let chat = chat.clone();
        flood.spawn(async move { session_entry(&chat, Uuid::now_v7()).await });
    }
    let found = tokio::time::timeout(Duration::from_secs(5), session_entry(&chat, real)).await?;
    assert_eq!(
        found.expect("a stored session").map(|entry| entry.user_id),
        Some("4242".to_owned())
    );
    while let Some(done) = flood.join_next().await {
        assert!(matches!(done?, Ok(None) | Err(ChatError::Busy { .. })));
    }

    let again = Uuid::now_v7();
    assert!(matches!(session_entry(&chat, again).await, Ok(None)));
    assert!(chat.callers.is_unknown(again));
    assert!(!chat.callers.is_unknown(real));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_stalled_feed_answers_a_flood_without_waiting_and_remembers_nothing(
    pool: PgPool,
) -> anyhow::Result<()> {
    let redis = crate::redis::connect_url("redis://127.0.0.1:1")?;
    let chat = chat_state(pool.clone(), redis, true);
    let real = stored_session(&pool, "4343").await?;
    stall_the_feed(&pool).await?;

    let started = std::time::Instant::now();
    let mut flood = tokio::task::JoinSet::new();
    for _ in 0..48 {
        let chat = chat.clone();
        flood.spawn(async move {
            let id = Uuid::now_v7();
            (id, session_entry(&chat, id).await)
        });
    }
    let found = session_entry(&chat, real).await;
    let mut answers = Vec::new();
    while let Some(done) = flood.join_next().await {
        answers.push(done?);
    }
    heal_the_feed(&pool).await?;

    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        found.expect("a stored session").map(|entry| entry.user_id),
        Some("4343".to_owned())
    );
    for (id, answer) in answers {
        let error = answer.expect_err("a stalled miss");
        assert!(matches!(
            error,
            ChatError::SessionNotReplicated | ChatError::Busy { .. }
        ));
        assert!(!chat.callers.is_unknown(id));
    }
    Ok(())
}
