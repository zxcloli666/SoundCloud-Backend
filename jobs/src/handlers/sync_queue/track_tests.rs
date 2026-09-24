use serde_json::{Value, json};
use sqlx::PgPool;
use std::time::Duration;

use super::repository::SyncQueueRepository;

async fn update(pool: &PgPool, body: &Value) -> anyhow::Result<()> {
    let patch = catalog_ingest::TrackUpdate::parse(body).map_err(anyhow::Error::msg)?;
    let mut tx = pool.begin().await?;
    sqlx::query_file!(
        "../api/queries/sync_queue/service/enqueue.sql",
        "17",
        "track_update",
        "soundcloud:tracks:42",
        body
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query_file_scalar!(
        "../api/queries/tracks/service/apply_update.sql",
        "42",
        "17",
        patch.desired()
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn setup(pool: &PgPool) -> anyhow::Result<SyncQueueRepository> {
    crate::db::migrations::run_core(pool, None).await?;
    sqlx::query("INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, uploader_sc_user_id)
        VALUES ('42', 'soundcloud:tracks:42', 'Track', 'track', 120000, '17')")
        .execute(pool).await?;
    update(pool, &json!({"track": {"title": "First edit"}})).await?;
    Ok(SyncQueueRepository::new(
        pool.clone(),
        Duration::from_secs(60),
    ))
}

#[sqlx::test(migrations = false)]
async fn newer_track_intent_fences_old_ack_and_confirmation_is_durable(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    let old = repository
        .claim(1)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("track mutation was not claimed"))?;
    assert!(repository.record_remote_attempt(&old).await?);
    assert!(
        repository
            .record_remote_success(&old, &json!({"urn": "soundcloud:tracks:42"}))
            .await?
    );
    update(&pool, &json!({"track": {"sharing": "private"}})).await?;
    repository.finalize(&old).await?;
    let row: (String, bool) =
        sqlx::query_as("SELECT sharing, sc_write_confirmed FROM tracks WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(row, ("private".into(), false));
    let current = repository
        .claim(1)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("track mutation was not claimed"))?;
    assert_eq!(current.generation, 2);
    assert!(repository.record_remote_attempt(&current).await?);
    assert!(
        repository
            .record_remote_success(&current, &json!({"urn": "soundcloud:tracks:42"}))
            .await?
    );
    repository.finalize(&current).await?;
    repository.finalize(&current).await?;
    let confirmed: bool =
        sqlx::query_scalar("SELECT sc_write_confirmed FROM tracks WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert!(confirmed);
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM sync_queue")
        .fetch_one(&pool)
        .await?;
    assert_eq!(pending, 0);
    let jobs: Vec<Value> =
        sqlx::query_scalar("SELECT payload FROM background_jobs WHERE kind = 'catalog.refresh'")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        jobs,
        [json!({"version": "1", "payload": {"entity": "track", "sc_id": "42", "owner_id": "17"}})]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn failed_confirmation_enqueue_preserves_the_stored_remote_result(
    pool: PgPool,
) -> anyhow::Result<()> {
    let repository = setup(&pool).await?;
    let mutation = repository
        .claim(1)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("track mutation was not claimed"))?;
    assert!(repository.record_remote_attempt(&mutation).await?);
    assert!(
        repository
            .record_remote_success(&mutation, &json!({"urn": "soundcloud:tracks:42"}))
            .await?
    );
    sqlx::query("ALTER TABLE background_jobs ADD CONSTRAINT no_confirmation CHECK (kind <> 'catalog.refresh')")
        .execute(&pool).await?;
    assert!(repository.finalize(&mutation).await.is_err());
    let confirmed: bool =
        sqlx::query_scalar("SELECT sc_write_confirmed FROM tracks WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert!(!confirmed);
    let completed: Option<i64> =
        sqlx::query_scalar("SELECT remote_completed_generation FROM sync_queue WHERE id = $1")
            .bind(mutation.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(completed, Some(mutation.generation));
    sqlx::query("ALTER TABLE background_jobs DROP CONSTRAINT no_confirmation")
        .execute(&pool)
        .await?;
    repository.finalize(&mutation).await?;
    Ok(())
}
