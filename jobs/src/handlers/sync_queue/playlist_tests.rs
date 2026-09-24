use serde_json::{Value, json};
use sqlx::PgPool;
use std::time::Duration;

use super::repository::SyncQueueRepository;

const PLAYLIST: &str = "soundcloud:playlists:42";

async fn update(pool: &PgPool, fields: Value) -> anyhow::Result<()> {
    let body = json!({"playlist": fields});
    let update = catalog_ingest::PlaylistUpdate::parse(&body).map_err(anyhow::Error::msg)?;
    let mut tx = pool.begin().await?;
    sqlx::query_file!(
        "../api/queries/sync_queue/service/enqueue.sql",
        "17",
        "playlist_update",
        PLAYLIST,
        &body
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query_file_scalar!(
        "../api/queries/playlists/service/apply_update.sql",
        PLAYLIST,
        "17",
        update.desired()
    )
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn seed_membership_state(pool: &PgPool, generation: i64) -> anyhow::Result<()> {
    let remote = json!({"urn": PLAYLIST, "title": "Mix", "sharing": "public", "user": {"urn": "soundcloud:users:17"}});
    catalog_ingest::upsert_playlist_from_sc(
        pool,
        &remote,
        catalog_ingest::Observation::begin(pool).await?,
    )
    .await?;
    sqlx::query(
        "INSERT INTO playlist_membership_state (playlist_urn, reconcile_generation, sync_status, next_reconcile_at)
         VALUES ($1, $2, 'shadow_ready', clock_timestamp() + interval '1 hour')
         ON CONFLICT (playlist_urn) DO UPDATE
         SET reconcile_generation = EXCLUDED.reconcile_generation,
             sync_status = EXCLUDED.sync_status,
             next_reconcile_at = EXCLUDED.next_reconcile_at",
    )
    .bind(PLAYLIST)
    .bind(generation)
    .execute(pool)
    .await?;
    Ok(())
}

async fn enqueue_membership(pool: &PgPool, generation: i64) -> anyhow::Result<()> {
    let payload = json!({
        "tracks": ["10", "20"],
        "fingerprint": "aabb",
        "reconcile_generation": generation
    });
    sqlx::query_file!(
        "../api/queries/sync_queue/service/enqueue.sql",
        "17",
        "playlist_membership",
        PLAYLIST,
        &payload
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn applied_mark(pool: &PgPool) -> anyhow::Result<(Option<String>, Option<i64>, bool)> {
    let row = sqlx::query_as::<_, (Option<String>, Option<i64>, bool)>(
        "SELECT encode(remote_apply_fingerprint, 'hex'),
                remote_apply_generation,
                next_reconcile_at <= clock_timestamp()
         FROM playlist_membership_state WHERE playlist_urn = $1",
    )
    .bind(PLAYLIST)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

async fn drive_membership(pool: &PgPool) -> anyhow::Result<()> {
    let repository = SyncQueueRepository::new(pool.clone(), Duration::from_secs(60));
    let mutation = repository
        .claim(1)
        .await?
        .into_iter()
        .find(|claimed| claimed.action_type == "playlist_membership")
        .ok_or_else(|| anyhow::anyhow!("missing membership mutation"))?;
    assert!(repository.record_remote_attempt(&mutation).await?);
    assert!(
        repository
            .record_remote_success(&mutation, &json!({"urn": PLAYLIST}))
            .await?
    );
    repository.finalize(&mutation).await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_finished_membership_apply_is_recorded_and_pulls_the_next_observation_forward(
    pool: PgPool,
) -> anyhow::Result<()> {
    crate::db::migrations::run_core(&pool, None).await?;
    seed_membership_state(&pool, 4).await?;
    enqueue_membership(&pool, 4).await?;

    drive_membership(&pool).await?;

    assert_eq!(
        applied_mark(&pool).await?,
        (Some("aabb".to_owned()), Some(4), true)
    );
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM sync_queue WHERE target_urn = $1")
        .bind(PLAYLIST)
        .fetch_one(&pool)
        .await?;
    assert_eq!(left, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_membership_apply_that_lost_its_generation_records_no_mark(
    pool: PgPool,
) -> anyhow::Result<()> {
    crate::db::migrations::run_core(&pool, None).await?;
    seed_membership_state(&pool, 9).await?;
    enqueue_membership(&pool, 4).await?;

    drive_membership(&pool).await?;

    assert_eq!(applied_mark(&pool).await?, (None, None, true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn playlist_metadata_ack_is_fenced_and_a_fresh_owner_read_confirms_the_patch(
    pool: PgPool,
) -> anyhow::Result<()> {
    crate::db::migrations::run_core(&pool, None).await?;
    let remote = json!({"urn": PLAYLIST, "title": "Original", "sharing": "public", "description": "Old", "user": {"urn": "soundcloud:users:17"}});
    catalog_ingest::upsert_playlist_from_sc(
        &pool,
        &remote,
        catalog_ingest::Observation::begin(&pool).await?,
    )
    .await?;
    update(&pool, json!({"title": "Renamed", "description": ""})).await?;
    let repository = SyncQueueRepository::new(pool.clone(), Duration::from_secs(60));
    let old = repository
        .claim(1)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing mutation"))?;
    assert!(repository.record_remote_attempt(&old).await?);
    assert!(
        repository
            .record_remote_success(&old, &json!({"urn": PLAYLIST}))
            .await?
    );
    update(&pool, json!({"sharing": "private"})).await?;
    repository.finalize(&old).await?;
    let confirmed: bool =
        sqlx::query_scalar("SELECT sc_write_confirmed FROM playlists WHERE urn = $1")
            .bind(PLAYLIST)
            .fetch_one(&pool)
            .await?;
    assert!(!confirmed);
    let current = repository
        .claim(1)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing current mutation"))?;
    assert!(repository.record_remote_attempt(&current).await?);
    assert!(
        repository
            .record_remote_success(&current, &json!({"urn": PLAYLIST}))
            .await?
    );
    repository.finalize(&current).await?;
    let jobs: Vec<Value> =
        sqlx::query_scalar("SELECT payload FROM background_jobs WHERE kind = 'catalog.refresh'")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        jobs,
        [
            json!({"version": "1", "payload": {"entity": "playlist", "sc_id": "42", "owner_id": "17"}})
        ]
    );
    let mut fresh = remote;
    fresh["title"] = json!("Renamed");
    fresh["description"] = Value::Null;
    fresh["sharing"] = json!("private");
    catalog_ingest::upsert_playlist_from_sc(
        &pool,
        &fresh,
        catalog_ingest::Observation::begin(&pool).await?,
    )
    .await?;
    let desired: Value = sqlx::query_scalar("SELECT sc_desired FROM playlists WHERE urn = $1")
        .bind(PLAYLIST)
        .fetch_one(&pool)
        .await?;
    assert_eq!(desired, json!({}));
    Ok(())
}
