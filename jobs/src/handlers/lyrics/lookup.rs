#[cfg(test)]
#[path = "lookup_tests.rs"]
mod db_tests;

use std::sync::Arc;
use std::time::Duration;

use backend_contracts::{JobKind, LyricsLookupPayload, StoredAudioDispatchPayload, Versioned};
use catalog_sources::{
    LyricsCandidate, LyricsFailure, LyricsHints, LyricsLookupOutcome, LyricsSources,
};
use chrono::{NaiveDate, Utc};
use futures::stream::{FuturesUnordered, StreamExt};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::config::LyricsConfig;
use crate::queue::{JobError, JobRepository, JobResult, LeasedJob, NewJob, QueueError};

use super::embedding_queue;
use super::text::lyrics_text;

const MAX_ATTEMPTS: i16 = 8;
const ALIGN_PRIORITY: i16 = 10;
const EXPIRED_RELEASE_BATCH: i64 = 256;
const INSUFFICIENT_METADATA_DELAY: i64 = 7 * 24 * 60 * 60;
const LOOKUP_DEADLINE: Duration = Duration::from_secs(180);

#[derive(sqlx::FromRow)]
struct LookupClaim {
    track_id: Uuid,
    sc_track_id: String,
    generation: i64,
    input_title: String,
    input_artist: String,
    input_duration_ms: i32,
    input_genius_song_id: Option<i64>,
    input_genius_url: Option<String>,
    input_release_date: Option<NaiveDate>,
    input_sc_created_at: Option<chrono::DateTime<Utc>>,
    miss_streak: i32,
    failure_streak: i32,
}

pub struct LyricsLookupHandler {
    pool: PgPool,
    queue: JobRepository,
    sources: Arc<LyricsSources>,
    config: LyricsConfig,
}

impl LyricsLookupHandler {
    pub fn new(pool: PgPool, sources: Arc<LyricsSources>, config: LyricsConfig) -> Self {
        Self {
            queue: JobRepository::new(pool.clone(), "lyrics-lookup".to_owned()),
            pool,
            sources,
            config,
        }
    }

    pub async fn run_targeted(&self, job: &LeasedJob, payload: LyricsLookupPayload) -> JobResult {
        let sc_track_id = canonical_track_id(&payload.sc_track_id).map_err(JobError::permanent)?;
        if job.dedup_key.as_deref() != Some(sc_track_id.as_str()) {
            return Err(JobError::permanent(anyhow::anyhow!(
                "lyrics lookup dedup key does not match its payload"
            )));
        }
        let claim = sqlx::query_file_as!(
            LookupClaim,
            "queries/lyrics/lookup_claim_target.sql",
            job.id,
            job.generation,
            job.lease_id,
            &sc_track_id,
            self.config.claim_seconds
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        if let Some(claim) = claim {
            self.process(job, claim).await?;
        }
        Ok(())
    }

    pub async fn run_sweep(&self, job: &LeasedJob) -> JobResult {
        if job.dedup_key.as_deref() != Some("schedule") {
            return Err(JobError::permanent(anyhow::anyhow!(
                "lyrics lookup sweep has an invalid dedup key"
            )));
        }
        sqlx::query_file!(
            "queries/lyrics/lookup_release_expired.sql",
            EXPIRED_RELEASE_BATCH
        )
        .execute(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        sqlx::query_file_scalar!(
            "queries/lyrics/lookup_backfill.sql",
            self.config.backfill_batch
        )
        .fetch_one(&self.pool)
        .await
        .map_err(JobError::retryable)?;

        let total = self.config.batch.max(1);
        let oldest = (total / 4).max(1).min(total);
        let priority = total - oldest;
        let mut claims = Vec::new();
        if priority > 0 {
            claims.extend(
                sqlx::query_file_as!(
                    LookupClaim,
                    "queries/lyrics/lookup_claim_priority.sql",
                    job.id,
                    job.generation,
                    job.lease_id,
                    priority,
                    self.config.claim_seconds
                )
                .fetch_all(&self.pool)
                .await
                .map_err(JobError::retryable)?,
            );
        }
        claims.extend(
            sqlx::query_file_as!(
                LookupClaim,
                "queries/lyrics/lookup_claim_oldest.sql",
                job.id,
                job.generation,
                job.lease_id,
                oldest,
                self.config.claim_seconds
            )
            .fetch_all(&self.pool)
            .await
            .map_err(JobError::retryable)?,
        );
        self.process_batch(job, claims).await
    }

    async fn process_batch(&self, job: &LeasedJob, claims: Vec<LookupClaim>) -> JobResult {
        let concurrency = self.config.concurrency.max(1);
        let mut queue = claims.into_iter();
        let mut running = FuturesUnordered::new();
        loop {
            while running.len() < concurrency
                && let Some(claim) = queue.next()
            {
                running.push(self.process(job, claim));
            }
            let Some(result) = running.next().await else {
                break;
            };
            result?;
        }
        Ok(())
    }

    async fn process(&self, job: &LeasedJob, claim: LookupClaim) -> JobResult {
        canonical_track_id(&claim.sc_track_id).map_err(JobError::permanent)?;
        let hints = LyricsHints {
            title: claim.input_title.clone(),
            artist: claim.input_artist.clone(),
            duration_sec: (claim.input_duration_ms > 0)
                .then(|| (claim.input_duration_ms as f64 / 1000.0).round() as i64),
            genius_song_id: claim.input_genius_song_id,
            genius_url: claim.input_genius_url.clone(),
        };
        let outcome = match tokio::time::timeout(LOOKUP_DEADLINE, self.sources.lookup(&hints)).await
        {
            Ok(outcome) => outcome,
            Err(_) => LyricsLookupOutcome::Retry(LyricsFailure {
                class: "deadline".to_owned(),
                retry_after_seconds: None,
            }),
        };
        match outcome {
            LyricsLookupOutcome::Found(candidate) => {
                self.settle_found(job, &claim, candidate).await
            }
            LyricsLookupOutcome::CleanMiss => {
                let delay = miss_delay_seconds(&claim);
                self.settle_miss(job, &claim, delay, "not_found").await
            }
            LyricsLookupOutcome::InsufficientMetadata => {
                self.settle_miss(
                    job,
                    &claim,
                    INSUFFICIENT_METADATA_DELAY,
                    "insufficient_metadata",
                )
                .await
            }
            LyricsLookupOutcome::Retry(failure) => self.settle_retry(job, &claim, failure).await,
        }
    }

    async fn settle_found(
        &self,
        job: &LeasedJob,
        claim: &LookupClaim,
        candidate: LyricsCandidate,
    ) -> JobResult {
        let synced_lrc = candidate.synced_lrc.unwrap_or_default();
        let plain_text = candidate.plain_text.unwrap_or_default();
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        let settled = sqlx::query_file!(
            "queries/lyrics/lookup_settle_found.sql",
            job.id,
            job.generation,
            job.lease_id,
            claim.track_id,
            claim.generation,
            &candidate.source,
            &synced_lrc,
            &plain_text
        )
        .fetch_optional(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
        let Some(settled) = settled else {
            transaction.commit().await.map_err(JobError::retryable)?;
            return Ok(());
        };
        if !settled.settled {
            transaction.rollback().await.map_err(JobError::retryable)?;
            return Err(JobError::retryable(anyhow::anyhow!(
                "lyrics lookup result was not fenced"
            )));
        }
        if lyrics_text(settled.plain_text.as_deref(), settled.synced_lrc.as_deref())
            .is_some_and(|text| text.chars().count() > 30)
        {
            embedding_queue::enqueue_if_new(&self.queue, &mut transaction, &settled.sc_track_id)
                .await?;
        }
        self.enqueue_align_if_ready(&mut transaction, &settled.sc_track_id)
            .await?;
        transaction.commit().await.map_err(JobError::retryable)?;
        Ok(())
    }

    async fn settle_miss(
        &self,
        job: &LeasedJob,
        claim: &LookupClaim,
        delay_seconds: i64,
        outcome: &str,
    ) -> JobResult {
        sqlx::query_file_scalar!(
            "queries/lyrics/lookup_settle_miss.sql",
            job.id,
            job.generation,
            job.lease_id,
            claim.track_id,
            claim.generation,
            delay_seconds,
            outcome
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        Ok(())
    }

    async fn settle_retry(
        &self,
        job: &LeasedJob,
        claim: &LookupClaim,
        failure: LyricsFailure,
    ) -> JobResult {
        let retry_after = failure
            .retry_after_seconds
            .and_then(|seconds| i64::try_from(seconds).ok())
            .unwrap_or_default();
        let delay = retry_delay_seconds(claim.failure_streak, retry_after);
        sqlx::query_file_scalar!(
            "queries/lyrics/lookup_settle_retry.sql",
            job.id,
            job.generation,
            job.lease_id,
            claim.track_id,
            claim.generation,
            delay,
            &failure.class,
            &failure.class,
            retry_after
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        Ok(())
    }

    async fn enqueue_align_if_ready(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        sc_track_id: &str,
    ) -> JobResult {
        let generation =
            sqlx::query_file_scalar!("queries/lyrics/lookup_align_candidate.sql", sc_track_id)
                .fetch_optional(&mut **transaction)
                .await
                .map_err(JobError::retryable)?;
        let Some(generation) = generation else {
            return Ok(());
        };
        let payload = StoredAudioDispatchPayload {
            sc_track_id: sc_track_id.to_owned(),
            uploaded_generation: generation,
        };
        let job = NewJob {
            id: Uuid::now_v7(),
            kind: JobKind::DispatchTranscription,
            dedup_key: Some(sc_track_id.to_owned()),
            payload: serde_json::to_value(Versioned::V1(payload)).map_err(JobError::permanent)?,
            priority: ALIGN_PRIORITY,
            max_attempts: MAX_ATTEMPTS,
            available_at: Utc::now(),
        };
        self.queue
            .enqueue_in_if_absent(transaction, &job)
            .await
            .map_err(queue_error)
    }
}

fn canonical_track_id(value: &str) -> anyhow::Result<String> {
    let value = value.strip_prefix("soundcloud:tracks:").unwrap_or(value);
    let id = value
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("lyrics lookup has an invalid track id"))?;
    if id == 0 || id.to_string() != value {
        anyhow::bail!("lyrics lookup has a non-canonical track id");
    }
    Ok(value.to_owned())
}

fn miss_delay_seconds(claim: &LookupClaim) -> i64 {
    const LADDER: [i64; 7] = [
        6 * 60 * 60,
        12 * 60 * 60,
        24 * 60 * 60,
        2 * 24 * 60 * 60,
        4 * 24 * 60 * 60,
        8 * 24 * 60 * 60,
        16 * 24 * 60 * 60,
    ];
    let index = usize::try_from(claim.miss_streak.max(0)).unwrap_or(usize::MAX);
    let mut delay = LADDER.get(index).copied().unwrap_or(30 * 24 * 60 * 60);
    let release_age_days = claim
        .input_release_date
        .map(|date| {
            Utc::now()
                .date_naive()
                .signed_duration_since(date)
                .num_days()
        })
        .or_else(|| {
            claim
                .input_sc_created_at
                .map(|created| Utc::now().signed_duration_since(created).num_days())
        });
    let young_release = release_age_days.is_some_and(|days| (0..30).contains(&days));
    let cap = if young_release {
        24 * 60 * 60
    } else {
        30 * 24 * 60 * 60
    };
    delay = delay.min(cap);
    delay
        .saturating_add(deterministic_jitter(
            claim.track_id,
            claim.miss_streak,
            delay,
        ))
        .min(cap)
}

fn retry_delay_seconds(failure_streak: i32, retry_after: i64) -> i64 {
    let exponent = u32::try_from(failure_streak.max(0)).unwrap_or(0).min(10);
    let exponential = 30_i64.saturating_mul(2_i64.saturating_pow(exponent));
    exponential
        .min(6 * 60 * 60)
        .max(retry_after.max(0))
        .min(24 * 60 * 60)
}

fn deterministic_jitter(track_id: Uuid, streak: i32, delay: i64) -> i64 {
    let bytes = track_id.as_bytes();
    let seed = u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]);
    let window = (delay / 5).max(1) as u64;
    let streak = u64::try_from(streak.max(0)).unwrap_or_default();
    i64::try_from(seed.rotate_left((streak % 64) as u32) % window).unwrap_or_default()
}

fn queue_error(error: QueueError) -> JobError {
    match error {
        QueueError::Database(error) => JobError::retryable(error),
        error => JobError::permanent(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(streak: i32) -> LookupClaim {
        LookupClaim {
            track_id: Uuid::from_u128(42),
            sc_track_id: "42".to_owned(),
            generation: 1,
            input_title: "Track".to_owned(),
            input_artist: "Artist".to_owned(),
            input_duration_ms: 180_000,
            input_genius_song_id: None,
            input_genius_url: None,
            input_release_date: None,
            input_sc_created_at: None,
            miss_streak: streak,
            failure_streak: 0,
        }
    }

    #[test]
    fn canonical_ids_accept_only_the_supported_forms() {
        assert_eq!(canonical_track_id("42").unwrap(), "42");
        assert_eq!(canonical_track_id("soundcloud:tracks:42").unwrap(), "42");
        for value in ["", "0", "042", "+42", "tracks:42", "foo:42"] {
            assert!(canonical_track_id(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn clean_miss_backoff_grows_and_caps() {
        let first = miss_delay_seconds(&claim(0));
        let second = miss_delay_seconds(&claim(1));
        let capped = miss_delay_seconds(&claim(99));

        assert!(first >= 6 * 60 * 60);
        assert!(second >= 12 * 60 * 60);
        assert_eq!(capped, 30 * 24 * 60 * 60);
    }

    #[test]
    fn retry_after_can_extend_but_not_escape_the_cap() {
        assert_eq!(retry_delay_seconds(0, 600), 600);
        assert_eq!(retry_delay_seconds(20, 48 * 60 * 60), 24 * 60 * 60);
    }

    #[test]
    fn jitter_is_stable_and_never_shortens_the_delay() {
        let claim = claim(4);
        let left = miss_delay_seconds(&claim);
        let right = miss_delay_seconds(&claim);

        assert_eq!(left, right);
        assert!(left >= 4 * 24 * 60 * 60);
    }
}
