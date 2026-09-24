mod ai;
mod coplay;
mod error;
mod genius_stage;
mod persist;
mod resolver;

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::stream::{FuturesUnordered, StreamExt};
use sqlx::PgPool;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::config::EnrichConfig;
use crate::handlers::external::ExternalSources;
use crate::queue::{JobError, JobResult};

use self::ai::AiResolverClient;
use self::error::{EnrichError, EnrichResult};
use self::resolver::{ResolveSource, ResolverDeps, TrackContext};

const LEASE_TIMEOUT: Duration = Duration::from_secs(120);
const BACKOFF_BASE: Duration = Duration::from_secs(5 * 60);
const BACKOFF_CAP: Duration = Duration::from_secs(6 * 60 * 60);

pub struct EnrichHandler {
    pool: PgPool,
    deps: Arc<ResolverDeps>,
    batch: i64,
    concurrency: usize,
    max_attempts: i16,
}

struct ClaimedTrack {
    id: Uuid,
    sc_track_id: String,
    enrich_attempts: i16,
}

struct EnrichTrack {
    id: Uuid,
    title: String,
    description: Option<String>,
    duration_ms: i32,
    isrc: Option<String>,
    metadata_artist: Option<String>,
    uploader_sc_user_id: Option<String>,
    uploader_username: Option<String>,
    enrich_source: Option<String>,
}

impl EnrichHandler {
    pub fn new(pool: PgPool, deps: ResolverDeps, config: &EnrichConfig) -> Self {
        let concurrency = config.concurrency.max(1);
        Self {
            pool,
            deps: Arc::new(deps),
            batch: (concurrency * 4) as i64,
            concurrency,
            max_attempts: config.max_attempts,
        }
    }

    pub async fn run(&self) -> JobResult {
        let claimed = self.claim().await.map_err(JobError::retryable)?;
        if claimed.is_empty() {
            return Ok(());
        }

        let mut enriched = 0usize;
        let mut failed = 0usize;
        let mut pending = FuturesUnordered::new();
        let mut queue = claimed.into_iter();

        loop {
            while pending.len() < self.concurrency
                && let Some(track) = queue.next()
            {
                pending.push(self.settle(track));
            }
            let Some(outcome) = pending.next().await else {
                break;
            };
            match outcome {
                Ok(true) => enriched += 1,
                Ok(false) => failed += 1,
                Err(error) => {
                    failed += 1;
                    warn!(%error, "enrichment bookkeeping failed");
                }
            }
        }

        if enriched > 0 || failed > 0 {
            info!(enriched, failed, "enrichment batch finished");
        }
        Ok(())
    }

    async fn settle(&self, track: ClaimedTrack) -> EnrichResult<bool> {
        match self.process(&track.sc_track_id).await {
            Ok(()) => Ok(true),
            Err(error) => {
                debug!(track = %track.sc_track_id, %error, "enrichment failed");
                self.record_failure(&track, &error.to_string()).await?;
                Ok(false)
            }
        }
    }

    async fn process(&self, sc_track_id: &str) -> EnrichResult<()> {
        let Some(track) = self.load(sc_track_id).await? else {
            return Ok(());
        };
        let context = TrackContext {
            title: track.title.clone(),
            uploader_username: track.uploader_username.clone(),
            uploader_sc_user_id: track.uploader_sc_user_id.clone(),
            duration_ms: Some(track.duration_ms),
            isrc: track.isrc.clone(),
            metadata_artist: track.metadata_artist.clone(),
            description: track.description.clone(),
        };

        let result = resolver::resolve_track(&context, &self.deps).await?;
        if result.primary.is_empty() {
            return Err(EnrichError::rejected(format!(
                "no primary artist resolved for {sc_track_id}"
            )));
        }
        if result.degraded {
            let previous = track.enrich_source.as_deref().unwrap_or("");
            if ResolveSource::priority_of(previous) > result.source.priority() {
                return Err(EnrichError::rejected(format!(
                    "transient source failure; keeping prior '{previous}' enrichment"
                )));
            }
        }

        let outcome = persist::apply(
            &self.pool,
            track.id,
            &result,
            track.uploader_sc_user_id.as_deref(),
            track.uploader_username.as_deref(),
        )
        .await?;
        if outcome.coplay_dirty
            && let Err(error) = coplay::recompute_for_track(&self.pool, track.id).await
        {
            warn!(track = %sc_track_id, %error, "coplay recompute failed");
        }
        debug!(
            track = %sc_track_id,
            primary = ?outcome.primary_artist_id,
            album = ?outcome.album_id,
            source = result.source.as_str(),
            confidence = result.confidence,
            "enriched"
        );
        Ok(())
    }

    async fn claim(&self) -> Result<Vec<ClaimedTrack>, sqlx::Error> {
        sqlx::query_file_as!(
            ClaimedTrack,
            "queries/enrich/source/claim_batch.sql",
            LEASE_TIMEOUT.as_secs() as f64,
            self.max_attempts,
            self.batch
        )
        .fetch_all(&self.pool)
        .await
    }

    async fn load(&self, sc_track_id: &str) -> EnrichResult<Option<EnrichTrack>> {
        let track = sqlx::query_file_as!(
            EnrichTrack,
            "queries/enrich/service/track_for_enrichment.sql",
            sc_track_id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(track)
    }

    async fn record_failure(&self, track: &ClaimedTrack, error: &str) -> EnrichResult<()> {
        let error: String = error.chars().take(300).collect();
        if track.enrich_attempts >= self.max_attempts {
            sqlx::query_file!(
                "queries/enrich/source/mark_dead.sql",
                track.id,
                Some(error.as_str())
            )
            .execute(&self.pool)
            .await?;
            return Ok(());
        }
        sqlx::query_file!(
            "queries/enrich/source/mark_failed.sql",
            track.id,
            next_run_after(track.enrich_attempts),
            Some(error.as_str())
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

pub fn build_resolver_deps(
    pool: PgPool,
    bus: crate::bus::Bus,
    sources: &ExternalSources,
    config: &EnrichConfig,
) -> ResolverDeps {
    let ai = config.ai_enabled.then(|| {
        Arc::new(AiResolverClient::new(
            bus,
            pool.clone(),
            config.ai_timeout_ms,
            config.ai_daily_budget,
        ))
    });
    ResolverDeps {
        mb: sources.musicbrainz.clone(),
        genius: sources.genius.clone(),
        ai,
        pg: pool,
    }
}

fn next_run_after(attempts: i16) -> DateTime<Utc> {
    let shift = attempts.clamp(0, 16) as u32;
    let seconds = BACKOFF_BASE
        .as_secs()
        .saturating_mul(1u64 << shift)
        .min(BACKOFF_CAP.as_secs());
    Utc::now() + chrono::Duration::seconds(seconds as i64)
}
