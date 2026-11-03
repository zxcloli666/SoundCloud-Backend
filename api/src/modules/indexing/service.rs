use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use sqlx::{PgConnection, PgPool};
use tracing::{debug, warn};

use crate::background_jobs::{BackgroundJob, BackgroundJobs, IndexingJobs};
use crate::cache::{LocalTtlCache, SingleFlight};
use crate::error::AppResult;
use crate::modules::tracks::TrackPriority;
use crate::modules::tracks::normalize::ScTrackFields;
use backend_contracts::{JobKind, LyricsLookupPayload};

#[derive(Debug, Clone, Serialize)]
pub struct IndexingStats {
    pub indexed: i64,
    pub pending: i64,
}

const STATS_TTL: Duration = Duration::from_secs(30);

pub struct IndexingService {
    pg: PgPool,
    background_jobs: Arc<BackgroundJobs>,
    indexing_jobs: Arc<IndexingJobs>,
    max_track_duration_ms: i32,
    stats_cache: LocalTtlCache<IndexingStats>,
    stats_flight: SingleFlight,
}

impl IndexingService {
    pub fn new(
        pg: PgPool,
        background_jobs: Arc<BackgroundJobs>,
        indexing_jobs: Arc<IndexingJobs>,
        max_track_duration_ms: i32,
    ) -> Arc<Self> {
        Arc::new(Self {
            pg,
            background_jobs,
            indexing_jobs,
            max_track_duration_ms,
            stats_cache: LocalTtlCache::new(STATS_TTL),
            stats_flight: SingleFlight::new(),
        })
    }

    pub async fn ingest_track_from_sc(
        self: &Arc<Self>,
        payload: &Value,
        priority: TrackPriority,
        observation: catalog_ingest::Observation,
    ) -> AppResult<()> {
        let Some(fields) = track_fields(payload) else {
            return Ok(());
        };
        let kick = wt::retry(|| async {
            let mut transaction = self.pg.begin().await?;
            let kick = self
                .ingest_in(&mut transaction, &fields, priority, observation)
                .await?;
            transaction.commit().await.map(|()| kick)
        })
        .await?;
        if kick {
            self.kick_pipeline(&fields.sc_track_id).await;
        }
        Ok(())
    }

    pub async fn ingest_upload(
        self: &Arc<Self>,
        payload: &Value,
        owned: &wt::OwnedInsert,
        observation: catalog_ingest::Observation,
    ) -> AppResult<()> {
        let fields = track_fields(payload);
        let kick = wt::retry(|| async {
            let mut transaction = self.pg.begin().await?;
            let kick = match &fields {
                Some(fields) => {
                    self.ingest_in(
                        &mut transaction,
                        fields,
                        TrackPriority::FreshDrop,
                        observation,
                    )
                    .await?
                }
                None => false,
            };
            wt::call(&mut transaction, owned).await?;
            transaction.commit().await.map(|()| kick)
        })
        .await?;
        if kick {
            self.kick_pipeline(&owned.sc_track_id).await;
        }
        Ok(())
    }

    async fn ingest_in(
        &self,
        connection: &mut PgConnection,
        fields: &ScTrackFields,
        priority: TrackPriority,
        observation: catalog_ingest::Observation,
    ) -> Result<bool, sqlx::Error> {
        let result =
            catalog_ingest::upsert_track_in(connection, fields, priority, priority, observation)
                .await?;
        if result.metadata_applied && self.is_too_long(fields.duration_ms) {
            mark_too_long(connection, &fields.sc_track_id).await?;
            return Ok(false);
        }
        Ok(result.was_new)
    }

    pub async fn release_held(&self, sc_track_id: &str) -> AppResult<bool> {
        let held = sqlx::query_file_scalar!("queries/indexing/service/is_held.sql", sc_track_id)
            .fetch_one(&self.pg)
            .await?;
        if !held {
            return Ok(false);
        }
        let release = wt::TrackReleaseHeld {
            sc_track_id: sc_track_id.to_owned(),
            at: wt::now(),
        };
        let released = wt::retry(|| async {
            let mut transaction = self.pg.begin().await?;
            let Some(duration_ms) = wt::call(&mut transaction, &release).await? else {
                return Ok(None);
            };
            let too_long = self.is_too_long(duration_ms);
            if too_long {
                mark_too_long(&mut transaction, sc_track_id).await?;
            }
            transaction.commit().await.map(|()| Some(too_long))
        })
        .await?;
        match released {
            None => Ok(false),
            Some(true) => Ok(true),
            Some(false) => {
                self.kick_pipeline(sc_track_id).await;
                Ok(true)
            }
        }
    }

    fn is_too_long(&self, duration_ms: i32) -> bool {
        self.max_track_duration_ms > 0 && duration_ms > self.max_track_duration_ms
    }

    async fn kick_pipeline(&self, sc_track_id: &str) {
        if let Err(error) = self.indexing_jobs.enqueue(sc_track_id).await {
            warn!(track = sc_track_id, %error, "indexing trigger enqueue failed");
        }
        let job = BackgroundJob::coalescing(
            JobKind::LyricsLookup,
            sc_track_id,
            LyricsLookupPayload {
                sc_track_id: sc_track_id.to_owned(),
            },
        )
        .map(|job| job.with_priority(10).if_absent());
        match job {
            Ok(job) => {
                if let Err(error) = self.background_jobs.enqueue(&job).await {
                    warn!(track = sc_track_id, %error, "lyrics lookup wake deferred to sweep");
                }
            }
            Err(error) => {
                warn!(track = sc_track_id, %error, "lyrics lookup job could not be created");
            }
        }
    }

    pub async fn get_stats(&self) -> AppResult<IndexingStats> {
        self.stats_flight
            .get_or_load(
                || async { self.stats_cache.get() },
                || async {
                    let row = sqlx::query_file!("queries/indexing/service/count_track_states.sql")
                        .fetch_one(&self.pg)
                        .await?;
                    let stats = IndexingStats {
                        indexed: row.indexed,
                        pending: row.total - row.indexed,
                    };
                    self.stats_cache.set(stats.clone());
                    Ok(stats)
                },
            )
            .await
    }
}

fn track_fields(payload: &Value) -> Option<ScTrackFields> {
    let fields = ScTrackFields::from_sc(payload);
    if fields.is_none() {
        debug!(
            urn = payload.get("urn").and_then(|v| v.as_str()).unwrap_or(""),
            title = payload.get("title").and_then(|v| v.as_str()).unwrap_or(""),
            "ingest skipped: ScTrackFields::from_sc returned None"
        );
    }
    fields
}

async fn mark_too_long(connection: &mut PgConnection, sc_track_id: &str) -> sqlx::Result<bool> {
    let too_long = wt::TrackTooLong {
        sc_track_id: sc_track_id.to_owned(),
        at: wt::now(),
    };
    wt::call(connection, &too_long).await
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    async fn seed(pool: &PgPool, indexed: i64, pending: i64) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, index_state)
             SELECT n::text, 'soundcloud:tracks:' || n, 'T' || n, 't' || n, 1000,
                    CASE WHEN n <= $1 THEN 'indexed' ELSE 'pending' END
             FROM generate_series(1, $2) AS n",
        )
        .bind(indexed)
        .bind(indexed + pending)
        .execute(pool)
        .await?;
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn stats_count_indexed_and_pending_in_one_pass(pool: PgPool) -> anyhow::Result<()> {
        seed(&pool, 3, 5).await?;

        let row = sqlx::query_file!("queries/indexing/service/count_track_states.sql")
            .fetch_one(&pool)
            .await?;

        assert_eq!(row.total, 8);
        assert_eq!(row.indexed, 3);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn repeated_stats_reads_do_not_rescan_the_track_table(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        seed(&pool, 2, 2).await?;
        let cache: LocalTtlCache<IndexingStats> = LocalTtlCache::new(STATS_TTL);
        let flight = SingleFlight::new();
        let scans = AtomicUsize::new(0);

        let load = || async {
            scans.fetch_add(1, Ordering::Relaxed);
            let row = sqlx::query_file!("queries/indexing/service/count_track_states.sql")
                .fetch_one(&pool)
                .await?;
            let stats = IndexingStats {
                indexed: row.indexed,
                pending: row.total - row.indexed,
            };
            cache.set(stats.clone());
            Ok::<_, sqlx::Error>(stats)
        };

        let first = flight.get_or_load(|| async { cache.get() }, load).await?;
        for _ in 0..20 {
            let again = flight.get_or_load(|| async { cache.get() }, load).await?;
            assert_eq!(again.indexed, first.indexed);
            assert_eq!(again.pending, first.pending);
        }

        assert_eq!(first.indexed, 2);
        assert_eq!(first.pending, 2);
        assert_eq!(
            scans.load(Ordering::Relaxed),
            1,
            "a warm stats read must answer from the cache instead of counting the whole track table again"
        );
        Ok(())
    }
}
