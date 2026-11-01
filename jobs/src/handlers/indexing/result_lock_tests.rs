use std::time::Duration;

use chrono::{TimeZone, Utc};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::*;

const NEIGHBOUR: &str = "100";
const CURRENT: &str = "200";
const FINGERPRINT: &str = "dup-fingerprint";

type TrackRow = (String, Option<Uuid>, Option<String>, bool);

async fn seed(pool: &PgPool, neighbour_canonical: Option<Uuid>) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms,
             storage_state, index_state, audio_fingerprint, canonical_track_id
         ) VALUES
             ($1, 'soundcloud:tracks:' || $1, 'Song', 'song', 180000,
              'ok', 'indexed', $3, $4),
             ($2, 'soundcloud:tracks:' || $2, 'Song', 'song', 180000,
              'ok', 'pending', NULL, NULL)",
    )
    .bind(NEIGHBOUR)
    .bind(CURRENT)
    .bind(FINGERPRINT)
    .bind(neighbour_canonical)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO storage_event_state (
             sc_track_id, stream, stream_sequence, event_published_at, uploaded_generation
         ) VALUES ($1, 'STORAGE_EVENTS', 1, to_timestamp(90), 1)",
    )
    .bind(CURRENT)
    .execute(pool)
    .await?;
    let dispatched = sqlx::query_file_scalar!(
        "queries/indexing/storage/dispatch_audio.sql",
        CURRENT,
        1_i64
    )
    .fetch_one(pool)
    .await?;
    anyhow::ensure!(dispatched.is_some(), "the current track was not dispatched");
    Ok(())
}

fn delivery() -> DeliveryContext {
    DeliveryContext {
        consumer: "backend-done-index-audio".to_owned(),
        stream: "PIPELINE_DONE".to_owned(),
        stream_sequence: 1,
        delivery_attempt: 1,
        published_at: Utc.timestamp_opt(201, 0).unwrap(),
    }
}

fn indexed() -> anyhow::Result<AudioIndex> {
    let result = AudioIndexResult {
        sc_track_id: CURRENT.to_owned(),
        upload_generation: 1,
        attempt: 1,
        status: WorkerStatus::Ok,
        reason: None,
        detail: None,
        producer: backend_contracts::pipeline::Producer {
            worker_id: "gpu-main".to_owned(),
            build: "test".to_owned(),
            models: std::collections::BTreeMap::new(),
            sync_version: None,
        },
        mert: Some(vec![0.25; TRACKS_MERT_DIMENSIONS as usize]),
        clap: Some(vec![0.5; TRACKS_CLAP_DIMENSIONS as usize]),
        fingerprint: Some(FINGERPRINT.to_owned()),
    };
    match validate_result(result)? {
        AudioResult::Indexed(index) => Ok(index),
        AudioResult::Settled { .. } => anyhow::bail!("an ok result must carry vectors"),
    }
}

fn handler(pool: &PgPool) -> anyhow::Result<AudioIndexResultHandler> {
    let qdrant = QdrantProvisioner::connect(&crate::config::QdrantConfig {
        grpc_url: "http://127.0.0.1:1".to_owned(),
        api_key: String::new().into(),
    })?;
    Ok(AudioIndexResultHandler::new(pool.clone(), qdrant))
}

fn spawn_commit(pool: &PgPool) -> tokio::task::JoinHandle<anyhow::Result<Committed>> {
    let pool = pool.clone();
    tokio::spawn(async move {
        let handler = handler(&pool)?;
        let index = indexed()?;
        let delivery = delivery();
        let ResultClaim::Owned { lease_id } = handler.claim(&index, 1, &delivery).await? else {
            anyhow::bail!("the indexing result did not own its claim");
        };
        Ok(handler.commit(&index, 1, &delivery, lease_id).await?)
    })
}

async fn wait_for_lock_wait(pool: &PgPool) -> anyhow::Result<()> {
    for _ in 0..400 {
        let waiting = sqlx::query_scalar::<_, i64>(
            "SELECT count(*)
             FROM pg_stat_activity
             WHERE datname = current_database()
               AND wait_event_type = 'Lock'",
        )
        .fetch_one(pool)
        .await?;
        if waiting > 0 {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    anyhow::bail!("the indexing result never waited on a row lock")
}

async fn s3_ok(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    sc_track_id: &str,
) -> anyhow::Result<()> {
    sqlx::query("SELECT public.scd_cmd_track_s3_state($1)")
        .bind(json!({"ok": [sc_track_id], "missing": [], "at": Utc::now()}))
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn rows(pool: &PgPool) -> anyhow::Result<Vec<TrackRow>> {
    Ok(sqlx::query_as(
        "SELECT sc_track_id, canonical_track_id, audio_fingerprint, s3_verified_at IS NOT NULL
         FROM tracks
         ORDER BY sc_track_id",
    )
    .fetch_all(pool)
    .await?)
}

#[sqlx::test(migrations = "../api/migrations")]
async fn an_indexing_result_and_a_sorted_s3_batch_never_deadlock(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, None).await?;
    let mut verdicts = pool.begin().await?;
    s3_ok(&mut verdicts, NEIGHBOUR).await?;

    let indexing = spawn_commit(&pool);
    wait_for_lock_wait(&pool).await?;
    s3_ok(&mut verdicts, CURRENT).await?;
    verdicts.commit().await?;
    let committed = indexing.await??;

    assert!(committed.committed);
    let rows = rows(&pool).await?;
    assert_eq!(rows.len(), 2);
    assert!(rows[0].1.is_some());
    assert_eq!(rows[0].1, rows[1].1);
    assert_eq!(rows[1].2.as_deref(), Some(FINGERPRINT));
    assert!(rows.iter().all(|row| row.3));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_neighbour_regrouped_before_its_lock_links_to_the_new_group(
    pool: PgPool,
) -> anyhow::Result<()> {
    let first_group = Uuid::now_v7();
    let second_group = Uuid::now_v7();
    seed(&pool, Some(first_group)).await?;
    let mut regroup = pool.begin().await?;
    sqlx::query("UPDATE tracks SET canonical_track_id = $2 WHERE sc_track_id = $1")
        .bind(NEIGHBOUR)
        .bind(second_group)
        .execute(&mut *regroup)
        .await?;

    let indexing = spawn_commit(&pool);
    wait_for_lock_wait(&pool).await?;
    regroup.commit().await?;
    let committed = indexing.await??;

    assert!(committed.committed);
    let rows = rows(&pool).await?;
    assert_eq!(rows[0].1, Some(second_group));
    assert_eq!(rows[1].1, Some(second_group));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_neighbour_that_stops_matching_before_its_lock_is_not_linked(
    pool: PgPool,
) -> anyhow::Result<()> {
    let group = Uuid::now_v7();
    seed(&pool, Some(group)).await?;
    let mut refingerprint = pool.begin().await?;
    sqlx::query("UPDATE tracks SET audio_fingerprint = $2 WHERE sc_track_id = $1")
        .bind(NEIGHBOUR)
        .bind("other-fingerprint")
        .execute(&mut *refingerprint)
        .await?;

    let indexing = spawn_commit(&pool);
    wait_for_lock_wait(&pool).await?;
    refingerprint.commit().await?;
    let committed = indexing.await??;

    assert!(committed.committed);
    let rows = rows(&pool).await?;
    assert_eq!(rows[0].1, Some(group));
    assert_eq!(rows[1].1, None);
    assert_eq!(rows[1].2.as_deref(), Some(FINGERPRINT));
    Ok(())
}

async fn record_track_writes(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE track_writes (n bigserial PRIMARY KEY, sc_track_id text NOT NULL);
         CREATE FUNCTION track_write() RETURNS trigger LANGUAGE plpgsql AS $f$
         BEGIN
             INSERT INTO track_writes (sc_track_id) VALUES (NEW.sc_track_id);
             RETURN NULL;
         END
         $f$;
         CREATE TRIGGER track_write AFTER UPDATE ON tracks
             FOR EACH ROW EXECUTE FUNCTION track_write();",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn tracks_written_by_commit(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    let handler = handler(pool)?;
    let index = indexed()?;
    let delivery = delivery();
    let ResultClaim::Owned { lease_id } = handler.claim(&index, 1, &delivery).await? else {
        anyhow::bail!("the indexing result did not own its claim");
    };
    let start: i64 = sqlx::query_scalar("SELECT coalesce(max(n), 0) FROM track_writes")
        .fetch_one(pool)
        .await?;
    let committed = handler.commit(&index, 1, &delivery, lease_id).await?;
    anyhow::ensure!(committed.committed, "the indexing result did not commit");
    Ok(
        sqlx::query_scalar("SELECT sc_track_id FROM track_writes WHERE n > $1 ORDER BY n")
            .bind(start)
            .fetch_all(pool)
            .await?,
    )
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_neighbour_the_link_changes_is_written_first_when_it_sorts_first(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, None).await?;
    record_track_writes(&pool).await?;

    let writes = tracks_written_by_commit(&pool).await?;

    assert_eq!(
        writes.first().map(String::as_str),
        Some(NEIGHBOUR),
        "{writes:?}"
    );
    let rows = rows(&pool).await?;
    assert!(rows[0].1.is_some());
    assert_eq!(rows[0].1, rows[1].1);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_neighbour_already_in_the_group_is_never_written(pool: PgPool) -> anyhow::Result<()> {
    let group = Uuid::now_v7();
    seed(&pool, Some(group)).await?;
    record_track_writes(&pool).await?;

    let writes = tracks_written_by_commit(&pool).await?;

    assert!(!writes.is_empty());
    assert!(writes.iter().all(|track| track == CURRENT), "{writes:?}");
    let rows = rows(&pool).await?;
    assert_eq!((rows[0].1, rows[1].1), (Some(group), Some(group)));
    Ok(())
}
