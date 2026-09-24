use anyhow::ensure;
use backend_contracts::pipeline::StorageTrackRejected;
use sqlx::PgPool;

use crate::bus::DeliveryContext;
use crate::queue::{JobError, JobResult};

const MAX_ATTEMPTS: i32 = 3;

pub struct StorageEventHandler {
    pool: PgPool,
}

struct RejectedTrack {
    sc_track_id: String,
    reason: &'static str,
    actual_secs: f64,
    expected_duration_ms: Option<i64>,
}

impl StorageEventHandler {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn reject(
        &self,
        payload: StorageTrackRejected,
        delivery: DeliveryContext,
    ) -> JobResult {
        let rejected = validate(payload).map_err(JobError::permanent)?;
        let stream_sequence = i64::try_from(delivery.stream_sequence)
            .map_err(|_| JobError::permanent(anyhow::anyhow!("NATS sequence is out of range")))?;
        let mut transaction = self.pool.begin().await.map_err(JobError::retryable)?;
        let outcome = sqlx::query_file!(
            "queries/indexing/storage/reject.sql",
            &delivery.consumer,
            &delivery.stream,
            stream_sequence,
            delivery.published_at,
            &rejected.sc_track_id,
            MAX_ATTEMPTS
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(JobError::retryable)?;

        if !outcome.known.unwrap_or(false) {
            transaction.rollback().await.map_err(JobError::retryable)?;
            return Err(JobError::retryable(anyhow::anyhow!(
                "storage rejection references unknown track {}",
                rejected.sc_track_id
            )));
        }
        if !outcome.accepted.unwrap_or(false) {
            transaction.commit().await.map_err(JobError::retryable)?;
            tracing::debug!(
                track = %rejected.sc_track_id,
                sequence = delivery.stream_sequence,
                "storage rejection was already applied"
            );
            return Ok(());
        }
        if !outcome.advanced.unwrap_or(false) {
            transaction.commit().await.map_err(JobError::retryable)?;
            tracing::debug!(
                track = %rejected.sc_track_id,
                sequence = delivery.stream_sequence,
                "stale storage rejection was ignored"
            );
            return Ok(());
        }
        if !outcome.updated.unwrap_or(false) {
            transaction.rollback().await.map_err(JobError::retryable)?;
            return Err(JobError::retryable(anyhow::anyhow!(
                "storage rejection cursor advanced without updating track {}",
                rejected.sc_track_id
            )));
        }
        transaction.commit().await.map_err(JobError::retryable)?;
        tracing::warn!(
            track = %rejected.sc_track_id,
            reason = rejected.reason,
            actual_secs = rejected.actual_secs,
            expected_duration_ms = ?rejected.expected_duration_ms,
            "storage rejected upload"
        );
        Ok(())
    }
}

fn validate(payload: StorageTrackRejected) -> anyhow::Result<RejectedTrack> {
    let sc_track_id = normalize_track_id(&payload.sc_track_id)?;
    let point_id = sc_track_id
        .parse::<u64>()
        .map_err(|_| anyhow::anyhow!("storage rejection has an invalid track id"))?;
    ensure!(
        point_id > 0 && point_id.to_string() == sc_track_id,
        "storage rejection has a non-canonical track id"
    );
    Ok(RejectedTrack {
        sc_track_id: sc_track_id.to_owned(),
        reason: payload.reason.as_str(),
        actual_secs: payload.actual_secs.unwrap_or(0.0),
        expected_duration_ms: payload.expected_duration_ms,
    })
}

fn normalize_track_id(value: &str) -> anyhow::Result<&str> {
    if let Some(track_id) = value.strip_prefix("soundcloud:tracks:") {
        return Ok(track_id);
    }
    ensure!(
        !value.contains(':'),
        "storage rejection has an invalid track id"
    );
    Ok(value)
}

#[cfg(test)]
mod tests {
    use backend_contracts::pipeline::StorageRejectionReason;
    use chrono::{TimeZone, Utc};

    use super::*;

    async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
        sqlx::raw_sql(
            "CREATE TABLE tracks (
                 sc_track_id text PRIMARY KEY,
                 storage_state varchar(16) NOT NULL,
                 storage_attempts smallint NOT NULL DEFAULT 0,
                 hq_upgrade_pending boolean NOT NULL DEFAULT false,
                 updated_at timestamptz NOT NULL DEFAULT now()
             );
             CREATE TABLE pipeline_event_receipts (
                 consumer varchar(96) NOT NULL,
                 stream varchar(128) NOT NULL,
                 stream_sequence bigint NOT NULL,
                 event_published_at timestamptz NOT NULL,
                 processed_at timestamptz NOT NULL DEFAULT now(),
                 PRIMARY KEY (consumer, stream, stream_sequence, event_published_at)
             );
             CREATE TABLE storage_event_state (
                 sc_track_id text PRIMARY KEY REFERENCES tracks(sc_track_id) ON DELETE CASCADE,
                 stream varchar(128) NOT NULL,
                 stream_sequence bigint NOT NULL,
                 event_published_at timestamptz NOT NULL,
                 uploaded_generation bigint NOT NULL DEFAULT 0,
                 updated_at timestamptz NOT NULL DEFAULT now()
             );",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    fn payload() -> StorageTrackRejected {
        StorageTrackRejected {
            sc_track_id: "42".to_owned(),
            reason: StorageRejectionReason::DurationMismatch,
            actual_secs: Some(12.5),
            expected_duration_ms: Some(10_000),
        }
    }

    fn delivery(sequence: u64) -> DeliveryContext {
        DeliveryContext {
            consumer: "backend-storage-rejected".to_owned(),
            stream: "STORAGE_EVENTS".to_owned(),
            stream_sequence: sequence,
            delivery_attempt: 1,
            published_at: Utc.timestamp_opt(1_700_000_000, 123_000_000).unwrap(),
        }
    }

    #[test]
    fn validation_rejects_non_canonical_track_ids() {
        let mut invalid = payload();
        invalid.sc_track_id = "tracks:42".to_owned();
        assert!(validate(invalid).is_err());
    }

    #[test]
    fn validation_normalizes_soundcloud_track_urns() {
        let mut urn = payload();
        urn.sc_track_id = "soundcloud:tracks:42".to_owned();
        assert_eq!(validate(urn).expect("valid URN").sc_track_id, "42");
    }

    #[sqlx::test(migrations = false)]
    async fn redelivery_applies_one_strike(pool: PgPool) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::query(
            "INSERT INTO tracks (
                 sc_track_id, storage_state, storage_attempts, hq_upgrade_pending
             ) VALUES ('42', 'pending', 0, true)",
        )
        .execute(&pool)
        .await?;
        let handler = StorageEventHandler::new(pool.clone());

        handler.reject(payload(), delivery(1)).await?;
        handler.reject(payload(), delivery(1)).await?;

        let state = sqlx::query_as::<_, (String, i16, bool)>(
            "SELECT storage_state, storage_attempts, hq_upgrade_pending
             FROM tracks WHERE sc_track_id = '42'",
        )
        .fetch_one(&pool)
        .await?;
        let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM pipeline_event_receipts")
            .fetch_one(&pool)
            .await?;
        assert_eq!(state, ("pending".to_owned(), 1, false));
        assert_eq!(receipts, 1);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn distinct_rejections_reach_the_failure_threshold(pool: PgPool) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::query(
            "INSERT INTO tracks (
                 sc_track_id, storage_state, storage_attempts, hq_upgrade_pending
             ) VALUES ('42', 'pending', 0, true)",
        )
        .execute(&pool)
        .await?;
        let handler = StorageEventHandler::new(pool.clone());

        for sequence in 1..=3 {
            handler.reject(payload(), delivery(sequence)).await?;
        }

        let state = sqlx::query_as::<_, (String, i16, bool)>(
            "SELECT storage_state, storage_attempts, hq_upgrade_pending
             FROM tracks WHERE sc_track_id = '42'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(state, ("failed".to_owned(), 3, false));
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn unknown_track_does_not_consume_the_delivery(pool: PgPool) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        let handler = StorageEventHandler::new(pool.clone());

        let error = handler
            .reject(payload(), delivery(1))
            .await
            .expect_err("unknown track must fail");

        let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM pipeline_event_receipts")
            .fetch_one(&pool)
            .await?;
        assert!(error.is_retryable());
        assert_eq!(receipts, 0);

        sqlx::query(
            "INSERT INTO tracks (
                 sc_track_id, storage_state, storage_attempts, hq_upgrade_pending
             ) VALUES ('42', 'pending', 0, true)",
        )
        .execute(&pool)
        .await?;
        handler.reject(payload(), delivery(1)).await?;
        let attempts: i16 =
            sqlx::query_scalar("SELECT storage_attempts FROM tracks WHERE sc_track_id = '42'")
                .fetch_one(&pool)
                .await?;
        assert_eq!(attempts, 1);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn rejection_does_not_downgrade_stored_audio(pool: PgPool) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::query(
            "INSERT INTO tracks (
                 sc_track_id, storage_state, storage_attempts, hq_upgrade_pending
             ) VALUES ('42', 'ok', 2, true)",
        )
        .execute(&pool)
        .await?;

        StorageEventHandler::new(pool.clone())
            .reject(payload(), delivery(1))
            .await?;

        let state = sqlx::query_as::<_, (String, i16, bool)>(
            "SELECT storage_state, storage_attempts, hq_upgrade_pending
             FROM tracks WHERE sc_track_id = '42'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(state, ("ok".to_owned(), 2, false));
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn stale_rejection_does_not_override_newer_storage_state(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::query(
            "INSERT INTO tracks (
                 sc_track_id, storage_state, storage_attempts, hq_upgrade_pending
             ) VALUES ('42', 'ok', 0, true)",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "INSERT INTO storage_event_state (
                 sc_track_id, stream, stream_sequence, event_published_at
             ) VALUES ('42', 'STORAGE_EVENTS', 10, to_timestamp(1800000000))",
        )
        .execute(&pool)
        .await?;
        let mut stale = delivery(9);
        stale.published_at = Utc.timestamp_opt(1_700_000_000, 0).unwrap();

        StorageEventHandler::new(pool.clone())
            .reject(payload(), stale)
            .await?;

        let state = sqlx::query_as::<_, (String, i16, bool)>(
            "SELECT storage_state, storage_attempts, hq_upgrade_pending
             FROM tracks WHERE sc_track_id = '42'",
        )
        .fetch_one(&pool)
        .await?;
        let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM pipeline_event_receipts")
            .fetch_one(&pool)
            .await?;
        assert_eq!(state, ("ok".to_owned(), 0, true));
        assert_eq!(receipts, 1);
        Ok(())
    }
}
