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
    let stalled = session_entry(&chat, missing).await;
    heal_the_feed(&pool).await?;

    let error = stalled.expect_err("a stalled feed must not answer a session");
    assert_eq!(error, ChatError::SessionNotReplicated);
    assert_eq!(error.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error.body()["error"], "chat_session_not_replicated");
    assert_eq!(error.body()["retry_ms"], 5_000);
    Ok(())
}
