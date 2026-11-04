use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use backend_contracts::{CatalogEntity, CatalogRefreshPayload, CollectionFailure, JobKind};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::CatalogRefreshHandler;
use crate::handlers::catalog_read::PublicCatalogReader;
use crate::handlers::catalog_refresh_writer::CatalogWriter;
use crate::handlers::catalog_remote::CatalogRemote;
use crate::handlers::soundcloud_stub::{GoneRelay, SoundCloud};
use crate::queue::{JobError, JobRepository, LeasedJob};

fn refresher(pool: &PgPool, public: PublicCatalogReader) -> anyhow::Result<CatalogRefreshHandler> {
    Ok(CatalogRefreshHandler {
        writer: CatalogWriter::new(pool.clone(), 420_000),
        public: Arc::new(public),
        remote: CatalogRemote::offline(pool.clone())?,
        pool: pool.clone(),
    })
}

fn track() -> CatalogRefreshPayload {
    CatalogRefreshPayload {
        entity: CatalogEntity::Track,
        sc_id: "42".into(),
        owner_id: None,
    }
}

async fn leased(pool: &PgPool) -> anyhow::Result<LeasedJob> {
    let job = LeasedJob {
        id: Uuid::now_v7(),
        kind: JobKind::CatalogRefresh,
        dedup_key: Some(track().dedup_key()),
        payload: Value::Null,
        generation: 1,
        attempts: 1,
        max_attempts: 8,
        lease_id: Uuid::now_v7(),
    };
    sqlx::query(
        "INSERT INTO background_jobs (
             id, kind, lane, dedup_key, payload, lease_id, lease_generation, generation,
             lease_expires_at, leased_by, attempts
         ) VALUES (
             $1, 'catalog.refresh', 'core_bulk', $2, '{}'::jsonb, $3, 1, 1,
             now() + interval '1 minute', 'refresh-test', 1
         )",
    )
    .bind(job.id)
    .bind(&job.dedup_key)
    .bind(job.lease_id)
    .execute(pool)
    .await?;
    Ok(job)
}

async fn refresh(pool: &PgPool, handler: &CatalogRefreshHandler) -> anyhow::Result<()> {
    let job = leased(pool).await?;
    match handler.refresh(&job, track()).await {
        Err(error @ JobError::Settled { .. }) => {
            JobRepository::new(pool.clone(), "refresh-test".to_owned())
                .fail(&job, &error.to_string(), false)
                .await?;
            Ok(())
        }
        Err(error) => Err(anyhow::anyhow!("refresh failed unsettled: {error}")),
        Ok(()) => {
            JobRepository::new(pool.clone(), "refresh-test".to_owned())
                .complete(&job)
                .await?;
            Ok(())
        }
    }
}

async fn settled(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT last_error FROM background_job_failures
         WHERE kind = 'catalog.refresh' AND dedup_key = 'track:42:public'
         ORDER BY failed_at",
    )
    .fetch_all(pool)
    .await?)
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_track_absent_in_every_region_settles_once_and_is_held_without_asking_again(
    pool: PgPool,
) -> anyhow::Result<()> {
    let soundcloud = SoundCloud::answering(Vec::new()).await?;
    let handler = refresher(&pool, soundcloud.reader(&pool)?)?;

    let job = leased(&pool).await?;
    let error = handler
        .refresh(&job, track())
        .await
        .err()
        .ok_or_else(|| anyhow::anyhow!("an absent track cannot be stored"))?;
    assert!(matches!(
        error,
        JobError::Settled {
            failure: CollectionFailure::NotFound,
            ..
        }
    ));
    assert_eq!(soundcloud.asked().await, 3);
    JobRepository::new(pool.clone(), "refresh-test".to_owned())
        .fail(&job, &error.to_string(), false)
        .await?;

    refresh(&pool, &handler).await?;
    refresh(&pool, &handler).await?;

    assert_eq!(soundcloud.asked().await, 3);
    assert_eq!(
        settled(&pool).await?,
        vec!["not_found: SoundCloud API error (status 404)".to_owned()]
    );
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(queued, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_held_absence_is_asked_again_once_its_day_is_over(pool: PgPool) -> anyhow::Result<()> {
    let soundcloud = SoundCloud::answering(Vec::new()).await?;
    let handler = refresher(&pool, soundcloud.reader(&pool)?)?;
    refresh(&pool, &handler).await?;
    sqlx::query("UPDATE background_job_failures SET failed_at = now() - interval '25 hours'")
        .execute(&pool)
        .await?;

    refresh(&pool, &handler).await?;

    assert_eq!(soundcloud.asked().await, 6);
    assert_eq!(settled(&pool).await?.len(), 2);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_track_one_region_hides_is_stored_and_never_settled(pool: PgPool) -> anyhow::Result<()> {
    let found = json!({
        "id": 42, "urn": "soundcloud:tracks:42", "title": "Found", "sharing": "public",
        "duration": 120000, "user": {"id": 7, "urn": "soundcloud:users:7", "username": "Owner"}
    });
    let soundcloud = SoundCloud::answering(vec![
        ("404 Not Found", "{}".to_owned()),
        ("200 OK", found.to_string()),
    ])
    .await?;
    let handler = refresher(&pool, soundcloud.reader(&pool)?)?;

    refresh(&pool, &handler).await?;

    assert_eq!(soundcloud.asked().await, 2);
    assert!(settled(&pool).await?.is_empty());
    let title: String = sqlx::query_scalar("SELECT title FROM tracks WHERE sc_track_id = '42'")
        .fetch_one(&pool)
        .await?;
    assert_eq!(title, "Found");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_track_the_relay_calls_gone_in_every_region_settles_without_the_proxy(
    pool: PgPool,
) -> anyhow::Result<()> {
    let soundcloud = SoundCloud::answering(Vec::new()).await?;
    let relayed = Arc::new(AtomicUsize::new(0));
    let sc = soundcloud.client()?.with_relay(Arc::new(GoneRelay {
        calls: Arc::clone(&relayed),
    }));
    let handler = refresher(&pool, PublicCatalogReader::new(sc, pool.clone()))?;

    refresh(&pool, &handler).await?;

    assert_eq!(relayed.load(Ordering::SeqCst), 3);
    assert_eq!(soundcloud.asked().await, 0);
    assert_eq!(
        settled(&pool).await?,
        vec!["not_found: SoundCloud API error (status 404)".to_owned()]
    );
    Ok(())
}
