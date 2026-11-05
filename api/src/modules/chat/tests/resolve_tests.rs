use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use super::session_entry;
use crate::modules::chat::ChatError;
use crate::modules::chat::caller::COLD_WAIT;
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

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_session_miss_during_a_replication_stall_asks_for_a_retry_not_a_login(
    pool: PgPool,
) -> anyhow::Result<()> {
    let redis = crate::redis::connect_url("redis://127.0.0.1:1")?;
    let chat = chat_state(pool.clone(), redis, true);
    let missing = Uuid::now_v7();

    let local = session_entry(&chat, missing, COLD_WAIT).await;
    assert!(matches!(local, Ok(None)), "{local:?}");

    stall_the_feed(&pool).await?;
    let missing = Uuid::now_v7();
    let stalled = session_entry(&chat, missing, COLD_WAIT).await;
    heal_the_feed(&pool).await?;

    let error = stalled.expect_err("a stalled feed must not answer a session");
    assert_eq!(error, ChatError::SessionNotReplicated);
    assert_eq!(error.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error.body()["error"], "chat_session_not_replicated");
    assert_eq!(error.body()["retry_ms"], 5_000);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_young_session_a_split_node_has_not_received_yet_asks_for_a_retry_not_a_login(
    pool: PgPool,
) -> anyhow::Result<()> {
    let split = split_without_replication(&pool).await?;
    let redis = crate::redis::connect_url("redis://127.0.0.1:1")?;
    let chat = chat_state(split.clone(), redis, true);
    let young = Uuid::now_v7();

    let missed = session_entry(&chat, young, COLD_WAIT).await;
    let unknown = chat.callers.is_unknown(young);
    land_session(&split, young, "4444").await?;
    let found = session_entry(&chat, young, COLD_WAIT).await;
    split.close().await;

    assert!(
        matches!(missed, Err(ChatError::SessionNotReplicated)),
        "{missed:?}"
    );
    assert!(
        !unknown,
        "a retryable miss must not be remembered as unknown"
    );
    assert_eq!(
        found
            .expect("the landed session")
            .map(|entry| entry.user_id),
        Some("4444".to_owned())
    );
    Ok(())
}

async fn split_without_replication(pool: &PgPool) -> anyhow::Result<PgPool> {
    sqlx::query(
        "DO $$ BEGIN EXECUTE format('ALTER DATABASE %I SET scd.mode = %L', current_database(), 'split'); END $$",
    )
    .execute(pool)
    .await?;
    Ok(PgPoolOptions::new()
        .max_connections(2)
        .connect_with((*pool.connect_options()).clone())
        .await?)
}

async fn land_session(pool: &PgPool, session: Uuid, user: &str) -> anyhow::Result<()> {
    let mut transaction = pool.begin().await?;
    sqlx::query("SET LOCAL scd.mode = 'legacy'")
        .execute(&mut *transaction)
        .await?;
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, $2, $2, 'access', 'refresh', now() + interval '1 hour', '')",
    )
    .bind(connection)
    .bind(user)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(session)
        .bind(connection)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
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

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_flood_of_random_session_ids_does_not_starve_a_real_user_and_unknown_ids_are_remembered(
    pool: PgPool,
) -> anyhow::Result<()> {
    let redis = crate::redis::connect_url("redis://127.0.0.1:1")?;
    let chat = chat_state(pool.clone(), redis, true);
    let real = stored_session(&pool, "4242").await?;

    let mut flood = tokio::task::JoinSet::new();
    for _ in 0..48 {
        let chat = chat.clone();
        flood.spawn(async move { session_entry(&chat, Uuid::now_v7(), COLD_WAIT).await });
    }
    let found = tokio::time::timeout(
        Duration::from_secs(5),
        session_entry(&chat, real, COLD_WAIT),
    )
    .await?;
    assert_eq!(
        found.expect("a stored session").map(|entry| entry.user_id),
        Some("4242".to_owned())
    );
    while let Some(done) = flood.join_next().await {
        assert!(matches!(done?, Ok(None) | Err(ChatError::Busy { .. })));
    }

    let again = Uuid::now_v7();
    assert!(matches!(
        session_entry(&chat, again, COLD_WAIT).await,
        Ok(None)
    ));
    assert!(chat.callers.is_unknown(again));
    assert!(!chat.callers.is_unknown(real));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
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
            (id, session_entry(&chat, id, COLD_WAIT).await)
        });
    }
    let found = session_entry(&chat, real, COLD_WAIT).await;
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
