use std::time::Duration;

use backend_contracts::JobKind;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use crate::config::JobScheduleConfig;
use crate::health::HealthState;

const POLL_INTERVAL: Duration = Duration::from_secs(1);
const CLAIM_BATCH: i64 = 32;
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

const SCHEDULES: &[Schedule] = &[
    Schedule::new(JobKind::ArtistAttributionRevalidate, 5 * 60, -10, 4).delayed(),
    Schedule::new(JobKind::CatalogWorkReconcile, 5 * 60, -10, 4).delayed(),
    Schedule::new(JobKind::CatalogCreditReview, 10 * 60, -10, 4).delayed(),
    Schedule::new(JobKind::AuthCleanupLinkRequests, 60, 0, 8),
    Schedule::new(JobKind::AuthCleanupLoginRequests, 60, 0, 8),
    Schedule::new(JobKind::CleanupJobReceipts, 5 * 60, -10, 4),
    Schedule::new(JobKind::CollabBootstrap, 60 * 60, 0, 8),
    Schedule::new(JobKind::CollabTrain, 6 * 60 * 60, -10, 4).delayed(),
    Schedule::new(JobKind::OAuthAppsRefresh, 60, 5, 8),
    Schedule::new(JobKind::DiscoverAccounts, 5 * 60, -10, 4).delayed(),
    Schedule::new(JobKind::DiscoverAggregates, 3 * 60 * 60, -5, 4),
    Schedule::new(JobKind::DiscoverCatalogGenius, 60, -10, 4).delayed(),
    Schedule::new(JobKind::DiscoverCatalogMusicBrainz, 60, -10, 4).delayed(),
    Schedule::new(JobKind::DiscoverInterest, 60 * 60, -10, 4),
    Schedule::new(JobKind::EnrichTracks, 30, 0, 8),
    Schedule::new(JobKind::IndexingReap, 5 * 60, 5, 8),
    Schedule::new(JobKind::LyricsReapEmbeddings, 10 * 60, 5, 8).delayed(),
    Schedule::new(JobKind::LyricsReapTranscriptions, 10 * 60, 5, 8).delayed(),
    Schedule::new(JobKind::LyricsLookupSweep, 60, -10, 4).delayed(),
    Schedule::new(JobKind::PlaylistReconcileSweep, 60, 0, 4).delayed(),
    Schedule::new(JobKind::PlaylistLegacyDrain, 5 * 60, -10, 4).delayed(),
    Schedule::new(JobKind::ResolveDurations, 60, 5, 8),
    Schedule::new(JobKind::ResolveWantedTracks, 60, -10, 4).delayed(),
    Schedule::new(JobKind::RecommendationColike, 6 * 60 * 60, -10, 4),
    Schedule::new(JobKind::RecommendationQualityBackfill, 10 * 60, -5, 8),
    Schedule::new(JobKind::RecommendationQualityTrain, 6 * 60 * 60, -10, 4),
    Schedule::new(JobKind::RecommendationWavePriority, 60 * 60, -5, 4),
    Schedule::new(JobKind::SubscriptionsSnapshot, 5 * 60, -5, 8),
    Schedule::new(JobKind::SyncQueueFlush, 60, 10, 8),
    Schedule::new(JobKind::SyncQueueHeal, 5 * 60, 0, 8),
];

#[derive(Clone, Copy)]
struct Schedule {
    kind: JobKind,
    interval_seconds: i32,
    priority: i16,
    max_attempts: i16,
    enabled: Option<bool>,
    initial_delay_seconds: i32,
}

impl Schedule {
    const fn new(kind: JobKind, interval_seconds: i32, priority: i16, max_attempts: i16) -> Self {
        Self {
            kind,
            interval_seconds,
            priority,
            max_attempts,
            enabled: None,
            initial_delay_seconds: 0,
        }
    }

    const fn delayed(mut self) -> Self {
        self.initial_delay_seconds = self.interval_seconds;
        self
    }
}

pub struct Scheduler {
    pool: PgPool,
    jitter_seed: u64,
    schedules: Vec<Schedule>,
}

impl Scheduler {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            jitter_seed: uuid::Uuid::new_v4().as_u128() as u64,
            schedules: SCHEDULES.to_vec(),
        }
    }

    pub fn configured(pool: PgPool, config: &JobScheduleConfig) -> Self {
        let mut scheduler = Self::new(pool);
        for schedule in &mut scheduler.schedules {
            match schedule.kind {
                JobKind::CollabTrain => {
                    schedule.interval_seconds = config.collab_train_seconds;
                    schedule.initial_delay_seconds = config.collab_train_seconds;
                    schedule.enabled = Some(config.collab_train_enabled);
                }
                JobKind::RecommendationColike => {
                    schedule.interval_seconds = config.recommendation_colike_seconds;
                }
                JobKind::ResolveDurations => {
                    schedule.interval_seconds = config.duration_resolver_seconds;
                }
                JobKind::RecommendationWavePriority => {
                    schedule.interval_seconds = config.recommendation_wave_priority_seconds;
                }
                JobKind::RecommendationQualityBackfill => {
                    schedule.interval_seconds = config.recommendation_quality_backfill_seconds;
                }
                JobKind::RecommendationQualityTrain => {
                    schedule.interval_seconds = config.recommendation_quality_train_seconds;
                }
                JobKind::DiscoverInterest => {
                    schedule.interval_seconds = config.discover_interest_seconds;
                    schedule.enabled = Some(config.discover_interest_enabled);
                }
                JobKind::EnrichTracks => {
                    schedule.enabled = Some(config.enrich_enabled);
                }
                JobKind::DiscoverCatalogGenius | JobKind::DiscoverCatalogMusicBrainz => {
                    schedule.interval_seconds = config.catalog_crawl_seconds;
                    schedule.initial_delay_seconds = config.catalog_crawl_seconds;
                    schedule.enabled = Some(config.catalog_crawl_enabled);
                }
                JobKind::ResolveWantedTracks => {
                    schedule.interval_seconds = config.wanted_resolve_seconds;
                    schedule.initial_delay_seconds = config.wanted_resolve_seconds;
                }
                JobKind::LyricsLookupSweep => {
                    schedule.interval_seconds = config.lyrics_lookup_seconds;
                    schedule.initial_delay_seconds = config.lyrics_lookup_seconds;
                }
                JobKind::PlaylistReconcileSweep => {
                    schedule.interval_seconds = config.playlist_reconcile_sweep_seconds;
                    schedule.initial_delay_seconds = config.playlist_reconcile_sweep_seconds;
                }
                _ => {}
            }
        }
        scheduler
    }

    pub async fn register(&self) -> Result<(), SchedulerError> {
        let mut transaction = self.pool.begin().await?;
        for schedule in &self.schedules {
            sqlx::query_file!(
                "queries/scheduler/register.sql",
                schedule.kind.as_str(),
                schedule.kind.lane().as_str(),
                schedule.interval_seconds,
                schedule.priority,
                schedule.max_attempts,
                schedule.enabled,
                schedule.initial_delay_seconds
            )
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn run(
        self,
        shutdown: CancellationToken,
        health: HealthState,
    ) -> Result<(), SchedulerError> {
        info!(
            schedules = self.schedules.len(),
            "background scheduler started"
        );
        let mut consecutive_failures = 0;

        loop {
            let result = tokio::select! {
                _ = shutdown.cancelled() => return Ok(()),
                result = self.enqueue_due() => result,
            };
            let delay = match result {
                Ok(enqueued) => {
                    consecutive_failures = 0;
                    health.mark_scheduler_success();
                    if enqueued > 0 {
                        debug!(enqueued, "scheduled jobs enqueued");
                    }
                    POLL_INTERVAL
                }
                Err(error) => {
                    consecutive_failures += 1;
                    let delay = retry_delay(consecutive_failures, self.jitter_seed);
                    tracing::warn!(
                        %error,
                        consecutive_failures,
                        retry_seconds = delay.as_secs_f64(),
                        "scheduler database operation will be retried"
                    );
                    delay
                }
            };
            tokio::select! {
                _ = shutdown.cancelled() => return Ok(()),
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }

    async fn enqueue_due(&self) -> Result<u64, SchedulerError> {
        let result = sqlx::query_file!("queries/scheduler/enqueue_due.sql", CLAIM_BATCH)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}

fn retry_delay(consecutive_failures: u32, seed: u64) -> Duration {
    let exponent = consecutive_failures.saturating_sub(1).min(5);
    let base = POLL_INTERVAL.saturating_mul(2_u32.saturating_pow(exponent));
    let jitter_window = base / 4;
    let jitter_millis = if jitter_window.is_zero() {
        0
    } else {
        seed.rotate_left(consecutive_failures) % (jitter_window.as_millis() as u64 + 1)
    };
    base.saturating_add(Duration::from_millis(jitter_millis))
        .min(MAX_RETRY_DELAY)
}

#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("scheduler database operation failed: {0}")]
    Database(#[from] sqlx::Error),
}

#[cfg(test)]
mod tests;
