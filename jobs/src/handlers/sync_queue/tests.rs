use std::time::Duration;

use serde_json::json;
use sqlx::PgPool;

use super::repository::SyncQueueRepository;

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE EXTENSION IF NOT EXISTS pgcrypto;
         CREATE TABLE sync_queue (
             id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
             user_id text NOT NULL,
             action_type varchar(32) NOT NULL,
             target_urn text NOT NULL,
             payload jsonb,
             locked_at timestamptz,
             retry_count integer NOT NULL DEFAULT 0,
             last_error text,
             next_run_at timestamptz NOT NULL DEFAULT now(),
             created_at timestamptz NOT NULL DEFAULT now(),
             dead boolean NOT NULL DEFAULT false,
             failed_at timestamptz
         );
         CREATE INDEX sync_queue_pickup_idx ON sync_queue (next_run_at, locked_at);
         CREATE UNIQUE INDEX sync_queue_target_uq
             ON sync_queue (user_id, action_type, target_urn);
         CREATE TABLE user_likes_tracks (
             user_id text NOT NULL,
             sc_track_id text NOT NULL,
             progress boolean NOT NULL DEFAULT false,
             synced_at timestamptz,
             created_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE user_likes_playlists (
             user_id text NOT NULL,
             playlist_urn text NOT NULL,
             progress boolean NOT NULL DEFAULT false,
             synced_at timestamptz,
             created_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE user_followings (
             user_id text NOT NULL,
             target_user_urn text NOT NULL,
             progress boolean NOT NULL DEFAULT false,
             synced_at timestamptz,
             created_at timestamptz NOT NULL DEFAULT now()
         );",
    )
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0068_sync_queue_leases.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0070_sync_queue_remote_attempts.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

fn repository(pool: PgPool) -> SyncQueueRepository {
    SyncQueueRepository::new(pool, Duration::from_secs(300))
}

#[sqlx::test(migrations = false)]
async fn comments_remain_distinct_while_state_actions_are_unique(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (user_id, action_type, target_urn)
         VALUES ('1', 'comment', 'track'), ('1', 'comment', 'track')",
    )
    .execute(&pool)
    .await?;

    let duplicate = sqlx::query(
        "INSERT INTO sync_queue (user_id, action_type, target_urn)
         VALUES ('1', 'like_track', 'track'), ('1', 'like_track', 'track')",
    )
    .execute(&pool)
    .await;

    assert!(duplicate.is_err());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn later_intent_waits_behind_a_head_in_backoff(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (
             user_id, action_type, target_urn, next_run_at, created_at
         ) VALUES
             ('1', 'like_track', 'track', now() + interval '1 hour', now()),
             ('1', 'unlike_track', 'track', now(), now() + interval '1 second')",
    )
    .execute(&pool)
    .await?;
    let repository = repository(pool.clone());

    assert!(repository.claim(2).await?.is_empty());

    sqlx::query("UPDATE sync_queue SET next_run_at = now() WHERE action_type = 'like_track'")
        .execute(&pool)
        .await?;
    let claimed = repository.claim(2).await?;
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].action_type, "like_track");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn terminal_intent_does_not_block_the_live_head(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (
             user_id, action_type, target_urn, created_at, dead, failed_at
         ) VALUES
             ('1', 'comment', 'track', now(), true, now()),
             ('1', 'like_track', 'track', now() + interval '1 second', false, NULL)",
    )
    .execute(&pool)
    .await?;
    let repository = repository(pool);

    let claimed = repository.claim(2).await?;

    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].action_type, "like_track");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn stale_remote_completion_cannot_delete_a_new_generation(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (user_id, action_type, target_urn)
         VALUES ('1', 'like_track', 'track')",
    )
    .execute(&pool)
    .await?;
    let repository = repository(pool.clone());
    let mutation = repository.claim(1).await?.remove(0);
    sqlx::query(
        "UPDATE sync_queue
         SET generation = generation + 1,
             remote_attempted_generation = NULL,
             remote_completed_generation = NULL,
             remote_result = NULL,
             next_run_at = now()
         WHERE id = $1",
    )
    .bind(mutation.id)
    .execute(&pool)
    .await?;

    assert!(
        !repository
            .record_remote_success(&mutation, &json!({ "ok": true }))
            .await?
    );
    repository.release(&mutation).await?;
    let state: (i64, bool) =
        sqlx::query_as("SELECT generation, lease_id IS NULL FROM sync_queue WHERE id = $1")
            .bind(mutation.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(state, (2, true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn forced_flush_preserves_recorded_remote_success(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (
             user_id, action_type, target_urn, next_run_at,
             remote_attempted_generation, remote_completed_generation, remote_result
         ) VALUES (
             '1', 'comment', 'track', now() + interval '1 hour', 1, 1, '{\"ok\":true}'
         )",
    )
    .execute(&pool)
    .await?;
    let repository = repository(pool.clone());

    repository.force_due().await?;

    let state: (i64, Option<i64>, bool) = sqlx::query_as(
        "SELECT generation, remote_completed_generation, remote_result IS NOT NULL
         FROM sync_queue",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(state, (1, Some(1), true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn recorded_comment_is_finalized_without_another_remote_call(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO sync_queue (user_id, action_type, target_urn, payload)
         VALUES ('1', 'comment', 'track', '{\"body\":\"hello\"}')",
    )
    .execute(&pool)
    .await?;
    let repository = repository(pool.clone());
    let mutation = repository.claim(1).await?.remove(0);
    assert!(repository.record_remote_attempt(&mutation).await?);
    repository
        .record_remote_success(&mutation, &json!({ "id": 1 }))
        .await?;

    repository.finalize(&mutation).await?;

    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM sync_queue")
        .fetch_one(&pool)
        .await?;
    assert_eq!(remaining, 0);
    Ok(())
}
