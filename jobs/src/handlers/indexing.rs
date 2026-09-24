mod client;
mod duration;
mod public_client;
mod result;
mod storage_events;
mod storage_uploaded;
#[cfg(test)]
mod test_schema;

use anyhow::anyhow;
use backend_contracts::pipeline::STORAGE_TRACK_UPLOADED;
use backend_contracts::worker_contract::AUDIO_LANE;
use backend_contracts::{IndexTrackPayload, JobKind, Versioned};
use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use url::Url;
use uuid::Uuid;

use crate::bus::Bus;
use crate::config::DurationConfig;
use crate::config::IndexingConfig;
use crate::qdrant::QdrantProvisioner;
use crate::queue::{JobError, JobRepository, JobResult, NewJob, QueueError};

use self::client::{IndexingClient, TriggerOutcome};
use self::duration::DurationResolver;
use self::result::AudioIndexResultHandler;
use self::storage_events::StorageEventHandler;
use self::storage_uploaded::StorageUploadHandler;

const REAP_BATCH: i64 = 50;
const FAILED_RETRY_BATCH: i64 = 10;
const REOPEN_BATCH: i64 = 50;
const MAX_ATTEMPTS: i16 = 8;
const MAX_DISPATCH_ATTEMPTS: i32 = MAX_ATTEMPTS as i32;
const AUDIO_INDEX_QUARANTINE_SECONDS: i64 = match AUDIO_LANE.quarantine_after_s() {
    Some(seconds) => seconds as i64,
    None => panic!("the audio lane must have an attempt window"),
};
const REAP_RETRY_COOLDOWN_SECONDS: i64 = 6 * 60 * 60;

pub struct IndexingHandler {
    pool: PgPool,
    queue: JobRepository,
    bus: Bus,
    client: IndexingClient,
    durations: DurationResolver,
    results: AudioIndexResultHandler,
    storage_events: StorageEventHandler,
    storage_uploads: StorageUploadHandler,
    storage_url: Url,
    audio_dispatch: bool,
}

impl IndexingHandler {
    pub fn new(
        pool: PgPool,
        config: &IndexingConfig,
        duration_config: &DurationConfig,
        storage_url: &Url,
        bus: Bus,
        qdrant: QdrantProvisioner,
    ) -> Result<Self, crate::ClientBuildError> {
        let queue = JobRepository::new(pool.clone(), "indexing-reap".to_owned());
        Ok(Self {
            queue: queue.clone(),
            pool: pool.clone(),
            bus: bus.clone(),
            client: IndexingClient::new(config)?,
            durations: DurationResolver::new(pool.clone(), queue, duration_config)?,
            results: AudioIndexResultHandler::new(pool.clone(), qdrant),
            storage_events: StorageEventHandler::new(pool.clone()),
            storage_uploads: StorageUploadHandler::new(
                pool.clone(),
                bus,
                storage_url.clone(),
                duration_config.max_track_duration_ms,
            ),
            storage_url: storage_url.clone(),
            audio_dispatch: false,
        })
    }

    pub fn with_audio_dispatch(mut self, enabled: bool) -> Self {
        self.audio_dispatch = enabled;
        self
    }

    pub async fn resolve_durations(&self) -> JobResult {
        self.durations.resolve_due().await
    }

    pub async fn finish_audio_index(
        &self,
        result: backend_contracts::pipeline::AudioIndexResult,
        delivery: crate::bus::DeliveryContext,
    ) -> JobResult {
        self.results.finish(result, delivery).await
    }

    pub async fn reject_storage(
        &self,
        rejection: backend_contracts::pipeline::StorageTrackRejected,
        delivery: crate::bus::DeliveryContext,
    ) -> JobResult {
        self.storage_events.reject(rejection, delivery).await
    }

    pub async fn accept_storage_upload(
        &self,
        upload: backend_contracts::pipeline::StorageTrackUploaded,
        delivery: crate::bus::DeliveryContext,
    ) -> JobResult {
        self.storage_uploads.accept(upload, delivery).await
    }

    pub async fn dispatch_audio(
        &self,
        payload: backend_contracts::StoredAudioDispatchPayload,
    ) -> JobResult {
        if !self.audio_dispatch {
            tracing::debug!(
                track = %payload.sc_track_id,
                generation = payload.uploaded_generation,
                "audio index dispatch is switched off"
            );
            return Ok(());
        }
        self.storage_uploads.dispatch_audio(payload).await
    }

    pub async fn apply_worker_lost(&self, stream_seq: u64, payload: &[u8]) -> JobResult {
        self.results.apply_worker_lost(stream_seq, payload).await
    }

    pub async fn index_track(&self, payload: IndexTrackPayload) -> JobResult {
        let sc_track_id = validate_track_id(&payload.sc_track_id)?;
        if !self.is_due(sc_track_id).await? {
            return Ok(());
        }

        match self.client.trigger(sc_track_id).await? {
            TriggerOutcome::Accepted => Ok(()),
            TriggerOutcome::Cached => self.finish_cached(sc_track_id).await,
        }
    }

    pub async fn reap(&self) -> JobResult {
        let settled = self.settle_unreopenable_dispatches().await;
        let reopened = self.reopen_dispatches().await;
        let requeued = self.requeue_stuck().await;
        settled.and(reopened).and(requeued)
    }

    async fn settle_unreopenable_dispatches(&self) -> JobResult {
        let quarantined = sqlx::query_file_scalar!(
            "queries/indexing/settle_unreopenable_dispatches.sql",
            REOPEN_BATCH,
            REAP_RETRY_COOLDOWN_SECONDS,
            MAX_DISPATCH_ATTEMPTS
        )
        .fetch_one(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        if quarantined > 0 {
            tracing::warn!(
                quarantined,
                "audio index attempts that cannot reopen were quarantined"
            );
        }
        Ok(())
    }

    async fn reopen_dispatches(&self) -> JobResult {
        if !self.audio_dispatch {
            return Ok(());
        }
        self.storage_uploads
            .reopen_audio_dispatches(
                REOPEN_BATCH,
                REAP_RETRY_COOLDOWN_SECONDS,
                MAX_DISPATCH_ATTEMPTS,
            )
            .await
    }

    async fn requeue_stuck(&self) -> JobResult {
        let stuck = sqlx::query_file_scalar!(
            "queries/indexing/reap_stuck.sql",
            REAP_BATCH,
            AUDIO_INDEX_QUARANTINE_SECONDS,
            REAP_RETRY_COOLDOWN_SECONDS,
            self.audio_dispatch
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        let failed = sqlx::query_file_scalar!(
            "queries/indexing/reap_failed.sql",
            FAILED_RETRY_BATCH,
            REAP_RETRY_COOLDOWN_SECONDS
        )
        .fetch_all(&self.pool)
        .await
        .map_err(JobError::retryable)?;

        for sc_track_id in stuck.into_iter().chain(failed) {
            self.enqueue(sc_track_id).await?;
        }
        Ok(())
    }

    async fn is_due(&self, sc_track_id: &str) -> JobResult<bool> {
        let row = sqlx::query_file!(
            "queries/indexing/track_state.sql",
            sc_track_id,
            AUDIO_INDEX_QUARANTINE_SECONDS
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(JobError::retryable)?;
        let Some(row) = row else {
            return Ok(false);
        };
        if row.index_state == "indexed"
            || row.index_state == "too_long"
            || row.storage_state == "too_long"
            || row.needs_duration_resolve
            || row.index_in_flight
            || (row.storage_state == "ok" && !self.audio_dispatch)
        {
            return Ok(false);
        }
        Ok(row.storage_state != "failed"
            || row.updated_at < Utc::now() - chrono::Duration::hours(24))
    }

    async fn finish_cached(&self, sc_track_id: &str) -> JobResult {
        let storage_url = self.storage_redirect_url(sc_track_id)?;
        self.bus
            .publish(
                STORAGE_TRACK_UPLOADED,
                &json!({
                    "sc_track_id": sc_track_id,
                    "storage_url": storage_url,
                }),
            )
            .await
            .map_err(JobError::retryable)
    }

    async fn enqueue(&self, sc_track_id: String) -> JobResult {
        let job = new_index_job(sc_track_id)?;
        self.queue.enqueue(&job).await.map_err(queue_error)
    }

    fn storage_redirect_url(&self, sc_track_id: &str) -> JobResult<String> {
        append_path(
            self.storage_url.clone(),
            &["redirect", &format!("soundcloud_tracks_{sc_track_id}.m4a")],
        )
        .map(Into::into)
    }
}

fn new_index_job(sc_track_id: String) -> JobResult<NewJob> {
    let payload = serde_json::to_value(Versioned::V1(IndexTrackPayload {
        sc_track_id: sc_track_id.clone(),
    }))
    .map_err(JobError::permanent)?;
    Ok(NewJob {
        id: Uuid::now_v7(),
        kind: JobKind::IndexTrack,
        dedup_key: Some(sc_track_id),
        payload,
        priority: 0,
        max_attempts: MAX_ATTEMPTS,
        available_at: Utc::now(),
    })
}

fn validate_track_id(sc_track_id: &str) -> JobResult<&str> {
    if !sc_track_id.is_empty() && sc_track_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Ok(sc_track_id);
    }
    Err(JobError::permanent(anyhow!("invalid SoundCloud track id")))
}

fn append_path(mut base: Url, segments: &[&str]) -> JobResult<Url> {
    let mut path = base
        .path_segments_mut()
        .map_err(|_| JobError::permanent(anyhow!("service URL cannot contain path segments")))?;
    path.extend(segments);
    drop(path);
    Ok(base)
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

    async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
        sqlx::raw_sql(
            "CREATE TABLE tracks (
                 sc_track_id text PRIMARY KEY,
                 storage_state varchar(16) NOT NULL,
                 index_state varchar(16) NOT NULL,
                 index_priority smallint NOT NULL DEFAULT 0,
                 needs_duration_resolve boolean NOT NULL DEFAULT false,
                 s3_verified_at timestamptz,
                 indexed_at timestamptz,
                 created_at timestamptz NOT NULL DEFAULT now() - interval '1 hour',
                 updated_at timestamptz NOT NULL DEFAULT now()
             );
             CREATE TABLE storage_event_state (
                 sc_track_id text PRIMARY KEY,
                 uploaded_generation bigint NOT NULL
             );
             CREATE TABLE background_jobs (
                 id uuid PRIMARY KEY,
                 kind varchar(96) NOT NULL,
                 dedup_key text
             );
             CREATE TABLE background_job_failures (
                 id uuid PRIMARY KEY,
                 kind varchar(96) NOT NULL,
                 dedup_key text,
                 failed_at timestamptz NOT NULL DEFAULT now()
             );
             INSERT INTO tracks (sc_track_id, storage_state, index_state, s3_verified_at)
             VALUES ('42', 'ok', 'pending', now());
             INSERT INTO storage_event_state (sc_track_id, uploaded_generation)
             VALUES ('42', 1);",
        )
        .execute(pool)
        .await?;
        test_schema::install_audio_index_wire_state(pool).await
    }

    async fn stuck(pool: &PgPool) -> anyhow::Result<Vec<String>> {
        stuck_while(pool, true).await
    }

    async fn stuck_while(pool: &PgPool, audio_dispatch: bool) -> anyhow::Result<Vec<String>> {
        Ok(sqlx::query_file_scalar!(
            "queries/indexing/reap_stuck.sql",
            REAP_BATCH,
            AUDIO_INDEX_QUARANTINE_SECONDS,
            REAP_RETRY_COOLDOWN_SECONDS,
            audio_dispatch
        )
        .fetch_all(pool)
        .await?)
    }

    async fn in_flight(pool: &PgPool) -> anyhow::Result<bool> {
        Ok(sqlx::query_file!(
            "queries/indexing/track_state.sql",
            "42",
            AUDIO_INDEX_QUARANTINE_SECONDS
        )
        .fetch_one(pool)
        .await?
        .index_in_flight)
    }

    #[test]
    fn a_dispatch_is_shielded_for_the_audio_lane_quarantine_window() {
        assert_eq!(
            AUDIO_INDEX_QUARANTINE_SECONDS,
            86_400 + 5 * 240 + 30 + 60 + 120 + 240
        );
    }

    #[test]
    fn track_ids_are_bare_decimal_values() -> anyhow::Result<()> {
        assert_eq!(validate_track_id("42")?, "42");
        for invalid in ["", " 42", "soundcloud:tracks:42", "track"] {
            assert!(validate_track_id(invalid).is_err());
        }
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn a_dispatched_index_is_not_retriggered_while_the_worker_owes_a_result(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        assert_eq!(stuck(&pool).await?, vec!["42".to_owned()]);
        assert!(!in_flight(&pool).await?);

        sqlx::query(
            "INSERT INTO audio_index_wire_state (
                 sc_track_id, status, upload_generation, dispatched_at
             ) VALUES ('42', 'pending', 1, now())",
        )
        .execute(&pool)
        .await?;

        assert!(stuck(&pool).await?.is_empty());
        assert!(in_flight(&pool).await?);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn an_abandoned_dispatch_stops_shielding_the_track(pool: PgPool) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::query(
            "INSERT INTO audio_index_wire_state (
                 sc_track_id, status, upload_generation, dispatched_at
             ) VALUES ('42', 'pending', 1, now() - interval '25 hours')",
        )
        .execute(&pool)
        .await?;

        assert_eq!(stuck(&pool).await?, vec!["42".to_owned()]);
        assert!(!in_flight(&pool).await?);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn an_answered_dispatch_keeps_shielding_the_track_until_quarantine(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::query(
            "INSERT INTO audio_index_wire_state (
                 sc_track_id, status, upload_generation, dispatched_at,
                 result_consumer, result_stream, result_stream_sequence, result_published_at,
                 updated_at
             ) VALUES (
                 '42', 'pending', 1, now() - interval '24 hours',
                 'backend-done-index-audio', 'PIPELINE_DONE', 1, now() - interval '23 hours',
                 now() - interval '23 hours'
             )",
        )
        .execute(&pool)
        .await?;

        assert!(stuck(&pool).await?.is_empty());
        assert!(in_flight(&pool).await?);

        sqlx::query(
            "UPDATE audio_index_wire_state
             SET dispatched_at = now() - make_interval(secs => $1::bigint + 60)",
        )
        .bind(AUDIO_INDEX_QUARANTINE_SECONDS)
        .execute(&pool)
        .await?;

        assert_eq!(stuck(&pool).await?, vec!["42".to_owned()]);
        assert!(!in_flight(&pool).await?);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn a_settled_generation_is_not_retriggered_by_the_stuck_reaper(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        for (status, reason) in [
            ("terminal", "undecodable_audio"),
            ("reopenable", "worker_lost"),
        ] {
            sqlx::query(
                "INSERT INTO audio_index_wire_state (
                     sc_track_id, status, upload_generation, dispatched_at, completed_at,
                     outcome_rank, outcome_status, outcome_reason
                 ) VALUES ('42', $1, 1, now() - interval '2 days', now(), 3, 'failed', $2)
                 ON CONFLICT (sc_track_id) DO UPDATE
                 SET status = EXCLUDED.status, outcome_reason = EXCLUDED.outcome_reason",
            )
            .bind(status)
            .bind(reason)
            .execute(&pool)
            .await?;

            assert!(stuck(&pool).await?.is_empty(), "{status}");
        }

        sqlx::query("UPDATE tracks SET storage_state = 'pending'")
            .execute(&pool)
            .await?;
        assert_eq!(stuck(&pool).await?, vec!["42".to_owned()]);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn a_settled_older_generation_does_not_shield_a_new_upload(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::query(
            "INSERT INTO audio_index_wire_state (
                 sc_track_id, status, upload_generation, dispatched_at, completed_at,
                 outcome_rank, outcome_status, outcome_reason
             ) VALUES (
                 '42', 'terminal', 1, now() - interval '2 days', now(),
                 3, 'failed', 'undecodable_audio'
             )",
        )
        .execute(&pool)
        .await?;
        let settled_current = stuck(&pool).await?;

        sqlx::query("UPDATE storage_event_state SET uploaded_generation = 2")
            .execute(&pool)
            .await?;

        assert!(settled_current.is_empty());
        assert_eq!(stuck(&pool).await?, vec!["42".to_owned()]);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn a_switched_off_dispatch_leaves_stored_tracks_alone_but_still_fetches_storage(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        install_schema(&pool).await?;

        assert!(stuck_while(&pool, false).await?.is_empty());

        sqlx::query("UPDATE tracks SET storage_state = 'pending'")
            .execute(&pool)
            .await?;
        assert_eq!(stuck_while(&pool, false).await?, vec!["42".to_owned()]);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn a_track_the_reaper_already_handed_out_does_not_take_a_slot_again(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        assert_eq!(stuck(&pool).await?, vec!["42".to_owned()]);

        sqlx::query(
            "INSERT INTO background_jobs (id, kind, dedup_key)
             VALUES (gen_random_uuid(), 'indexing.track', '42')",
        )
        .execute(&pool)
        .await?;

        assert!(stuck(&pool).await?.is_empty());
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn a_track_that_just_exhausted_its_attempts_waits_out_the_cooldown(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::query(
            "INSERT INTO background_job_failures (id, kind, dedup_key, failed_at)
             VALUES (gen_random_uuid(), 'indexing.track', '42', now())",
        )
        .execute(&pool)
        .await?;

        assert!(stuck(&pool).await?.is_empty());

        sqlx::query("UPDATE background_job_failures SET failed_at = now() - interval '7 hours'")
            .execute(&pool)
            .await?;

        assert_eq!(stuck(&pool).await?, vec!["42".to_owned()]);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn a_quarantined_dispatch_does_not_shield_the_track(pool: PgPool) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::query(
            "INSERT INTO audio_index_wire_state (
                 sc_track_id, status, upload_generation, dispatched_at, quarantine_reason
             ) VALUES ('42', 'quarantined', 1, now(), 'new_upload_during_pending')",
        )
        .execute(&pool)
        .await?;

        assert_eq!(stuck(&pool).await?, vec!["42".to_owned()]);
        assert!(!in_flight(&pool).await?);
        Ok(())
    }
}
