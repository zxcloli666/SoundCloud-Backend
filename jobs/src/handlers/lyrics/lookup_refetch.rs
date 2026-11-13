use catalog_sources::{GeniusTexts, LyricsFailure, LyricsHints};
use sqlx::PgExecutor;

use super::{EXPIRED_RELEASE_BATCH, LOOKUP_DEADLINE, LyricsLookupHandler};
use crate::handlers::lyrics::refetch_text::{Verdict, verdict};
use crate::handlers::star_priority;
use crate::queue::{JobError, JobResult, LeasedJob};

pub(super) const REFETCH_MAX_ATTEMPTS: i32 = 6;
const RETRY_BASE_SECONDS: i64 = 60 * 60;
const RETRY_CAP_SECONDS: i64 = 24 * 60 * 60;

#[derive(sqlx::FromRow)]
pub(super) struct RefetchClaim {
    pub(super) sc_track_id: String,
    pub(super) attempts: i32,
    pub(super) input_title: Option<String>,
    pub(super) input_artist: Option<String>,
    pub(super) input_duration_ms: Option<i32>,
    pub(super) input_genius_song_id: Option<i64>,
    pub(super) input_genius_url: Option<String>,
}

impl RefetchClaim {
    fn hints(&self) -> Option<LyricsHints> {
        let title = self.input_title.as_deref()?.trim();
        if title.is_empty() {
            return None;
        }
        Some(LyricsHints {
            title: title.to_owned(),
            artist: self.input_artist.clone().unwrap_or_default(),
            duration_sec: self
                .input_duration_ms
                .filter(|duration| *duration > 0)
                .map(|duration| (f64::from(duration) / 1000.0).round() as i64),
            genius_song_id: self.input_genius_song_id,
            genius_url: self.input_genius_url.clone(),
        })
    }
}

impl LyricsLookupHandler {
    pub(super) async fn release_expired_refetches(&self) -> JobResult {
        let released = sqlx::query_file_scalar!(
            "queries/lyrics/refetch_release_expired.sql",
            EXPIRED_RELEASE_BATCH,
            REFETCH_MAX_ATTEMPTS
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        let abandoned = released.iter().filter(|status| *status == "done").count();
        crate::metrics::record_lyrics_refetch("abandoned", abandoned as u64);
        Ok(())
    }

    pub(super) async fn process_refetch(&self, job: &LeasedJob, claim: RefetchClaim) -> JobResult {
        let Some(hints) = claim.hints() else {
            return self.keep(&self.pool, job, &claim, "no_metadata").await;
        };
        let texts =
            match tokio::time::timeout(LOOKUP_DEADLINE, self.sources.genius_texts(&hints)).await {
                Ok(texts) => texts,
                Err(_) => GeniusTexts::Retry(LyricsFailure {
                    class: "deadline".to_owned(),
                    retry_after_seconds: None,
                }),
            };
        self.settle_refetch(job, &claim, texts).await
    }

    pub(super) async fn settle_refetch(
        &self,
        job: &LeasedJob,
        claim: &RefetchClaim,
        texts: GeniusTexts,
    ) -> JobResult {
        match texts {
            GeniusTexts::Missing => self.keep(&self.pool, job, claim, "not_found").await,
            GeniusTexts::InsufficientMetadata => {
                self.keep(&self.pool, job, claim, "no_metadata").await
            }
            GeniusTexts::Retry(_) => self.retry_refetch(job, claim).await,
            GeniusTexts::Found(texts) => self.settle_texts(job, claim, &texts).await,
        }
    }

    async fn retry_refetch(&self, job: &LeasedJob, claim: &RefetchClaim) -> JobResult {
        if claim.attempts >= REFETCH_MAX_ATTEMPTS {
            return self.keep(&self.pool, job, claim, "failed").await;
        }
        let exponent = u32::try_from(claim.attempts.max(1) - 1).unwrap_or(0);
        let delay = RETRY_BASE_SECONDS
            .saturating_mul(2_i64.saturating_pow(exponent))
            .min(RETRY_CAP_SECONDS);
        let updated = self
            .mark(&self.pool, job, claim, "pending", "", delay)
            .await?;
        if updated {
            crate::metrics::record_lyrics_refetch("retry", 1);
        }
        Ok(())
    }

    async fn settle_texts(
        &self,
        job: &LeasedJob,
        claim: &RefetchClaim,
        texts: &[catalog_sources::GeniusText],
    ) -> JobResult {
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        let locked = sqlx::query_file!(
            "queries/lyrics/refetch_lock.sql",
            job.id,
            job.generation,
            job.lease_id,
            &claim.sc_track_id
        )
        .fetch_optional(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;
        let Some(locked) = locked else {
            transaction.commit().await.map_err(JobError::retryable)?;
            return Ok(());
        };
        let outcome = match locked.content_generation.filter(|_| locked.same_row) {
            None => self.keep(&mut *transaction, job, claim, "stale").await,
            Some(generation) => {
                let stored = locked.plain_text.as_deref().unwrap_or_default();
                match verdict(stored, texts) {
                    Verdict::Replace(index) => {
                        self.replace(&mut transaction, job, claim, generation, &texts[index].text)
                            .await
                    }
                    Verdict::Unchanged => {
                        self.keep(&mut *transaction, job, claim, "unchanged").await
                    }
                    Verdict::Different => {
                        self.keep(&mut *transaction, job, claim, "different").await
                    }
                }
            }
        };
        outcome?;
        transaction.commit().await.map_err(JobError::retryable)
    }

    async fn replace(
        &self,
        transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        job: &LeasedJob,
        claim: &RefetchClaim,
        generation: i64,
        text: &str,
    ) -> JobResult {
        let settled = sqlx::query_file_scalar!(
            "queries/lyrics/refetch_replace.sql",
            job.id,
            job.generation,
            job.lease_id,
            &claim.sc_track_id,
            generation,
            text
        )
        .fetch_optional(&mut **transaction)
        .await
        .map_err(JobError::retryable)?;
        let Some(outcome) = settled else {
            return Ok(());
        };
        if outcome != "replaced" {
            crate::metrics::record_lyrics_refetch("stale", 1);
            return Ok(());
        }
        let tier = star_priority::of_track(&mut **transaction, &claim.sc_track_id)
            .await
            .map_err(JobError::retryable)?;
        self.enqueue_align_if_ready(transaction, &claim.sc_track_id, tier)
            .await?;
        crate::metrics::record_lyrics_refetch("replaced", 1);
        Ok(())
    }

    async fn keep<'e>(
        &self,
        executor: impl PgExecutor<'e>,
        job: &LeasedJob,
        claim: &RefetchClaim,
        outcome: &'static str,
    ) -> JobResult {
        if self.mark(executor, job, claim, "done", outcome, 0).await? {
            crate::metrics::record_lyrics_refetch(outcome, 1);
        }
        Ok(())
    }

    async fn mark<'e>(
        &self,
        executor: impl PgExecutor<'e>,
        job: &LeasedJob,
        claim: &RefetchClaim,
        status: &str,
        outcome: &str,
        delay_seconds: i64,
    ) -> Result<bool, JobError> {
        let updated = sqlx::query_file_scalar!(
            "queries/lyrics/refetch_settle.sql",
            job.id,
            job.generation,
            job.lease_id,
            &claim.sc_track_id,
            status,
            outcome,
            delay_seconds
        )
        .fetch_optional(executor)
        .await
        .map_err(JobError::retryable)?;
        Ok(updated.is_some())
    }
}
