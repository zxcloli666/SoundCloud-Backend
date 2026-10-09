use backend_contracts::{HardNegative, ImpressionBatch};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::queue::{JobError, JobResult, LeasedJob};

use super::payload;

const MAX_IMPRESSIONS: usize = 2_048;
const MAX_FEATURES: usize = 512;
const IMPRESSION_WAIT_MINUTES: i64 = 10;
const IMPRESSION_WAIT_SECONDS: i64 = IMPRESSION_WAIT_MINUTES * 60;
const SWEEP_BATCH: usize = 2_000;
const SWEEP_BATCH_LIMIT: i64 = SWEEP_BATCH as i64;
const SWEEP_BATCHES_PER_RUN: usize = 50;
const SWEEP_FIRST_LOOKBACK_SECONDS: i64 = 24 * 60 * 60;

pub struct TelemetryHandler {
    impressions: PgPool,
    hard_negatives: PgPool,
    events: PgPool,
}

struct SweepCursor {
    scanned_through: NaiveDateTime,
    scanned_event_id: Uuid,
}

struct EarlySkip {
    event_id: Uuid,
    sc_user_id: String,
    sc_track_id: String,
    position_pct: f32,
    created_at: NaiveDateTime,
    detected_at: DateTime<Utc>,
}

impl TelemetryHandler {
    pub fn new(impressions: PgPool, hard_negatives: PgPool, events: PgPool) -> Self {
        Self {
            impressions,
            hard_negatives,
            events,
        }
    }

    pub async fn sweep_hard_negatives(&self) -> JobResult {
        let mut cursor = sqlx::query_file_as!(
            SweepCursor,
            "queries/telemetry/hard_negative_sweep_cursor.sql",
            SWEEP_FIRST_LOOKBACK_SECONDS + IMPRESSION_WAIT_SECONDS
        )
        .fetch_one(&self.hard_negatives)
        .await
        .map_err(JobError::retryable)?;
        let mut scanned = 0usize;
        let mut recorded = 0i64;
        for _ in 0..SWEEP_BATCHES_PER_RUN {
            let skips = sqlx::query_file_as!(
                EarlySkip,
                "queries/telemetry/hard_negative_candidates.sql",
                cursor.scanned_through,
                cursor.scanned_event_id,
                IMPRESSION_WAIT_SECONDS,
                SWEEP_BATCH_LIMIT
            )
            .fetch_all(&self.events)
            .await
            .map_err(JobError::retryable)?;
            let Some(last) = skips.last() else {
                break;
            };
            let next = SweepCursor {
                scanned_through: last.created_at,
                scanned_event_id: last.event_id,
            };
            recorded += self.record_early_skips(&skips, &next).await?;
            scanned += skips.len();
            cursor = next;
            if skips.len() < SWEEP_BATCH {
                break;
            }
        }
        if scanned > 0 {
            tracing::info!(scanned, recorded, "early skips swept into hard negatives");
        }
        Ok(())
    }

    async fn record_early_skips(&self, skips: &[EarlySkip], next: &SweepCursor) -> JobResult<i64> {
        let event_ids: Vec<_> = skips.iter().map(|skip| skip.event_id).collect();
        let user_ids: Vec<_> = skips.iter().map(|skip| skip.sc_user_id.clone()).collect();
        let track_ids: Vec<_> = skips.iter().map(|skip| skip.sc_track_id.clone()).collect();
        let positions: Vec<_> = skips.iter().map(|skip| skip.position_pct).collect();
        let detected_at: Vec<_> = skips.iter().map(|skip| skip.detected_at).collect();
        sqlx::query_file_scalar!(
            "queries/telemetry/record_hard_negatives.sql",
            &event_ids,
            &user_ids,
            &track_ids,
            &positions,
            &detected_at,
            next.scanned_through,
            next.scanned_event_id
        )
        .fetch_one(&self.hard_negatives)
        .await
        .map_err(JobError::retryable)
    }

    pub async fn record_impressions(&self, batch: &ImpressionBatch) -> JobResult {
        validate_batch(batch)?;

        let event_ids: Vec<_> = batch
            .impressions
            .iter()
            .map(|item| item.impression_id)
            .collect();
        let request_ids = vec![batch.request_id; batch.impressions.len()];
        let user_ids: Vec<_> = batch
            .impressions
            .iter()
            .map(|item| item.user_id.clone())
            .collect();
        let track_ids: Vec<_> = batch
            .impressions
            .iter()
            .map(|item| item.track_id.clone())
            .collect();
        let cluster_ids: Vec<_> = batch
            .impressions
            .iter()
            .map(|item| item.cluster_id.clone())
            .collect();
        let sources: Vec<_> = batch
            .impressions
            .iter()
            .map(|item| item.source.clone())
            .collect();
        let positions: Vec<_> = batch.impressions.iter().map(|item| item.position).collect();
        let scores_present: Vec<_> = batch
            .impressions
            .iter()
            .map(|item| item.score.is_some())
            .collect();
        let scores: Vec<_> = batch
            .impressions
            .iter()
            .map(|item| item.score.unwrap_or_default())
            .collect();
        let features: Vec<Value> = batch
            .impressions
            .iter()
            .map(|item| item.features.clone().map_or(Value::Null, Value::from))
            .collect();
        let shown_at: Vec<_> = batch
            .impressions
            .iter()
            .map(|item| timestamp(item.shown_at_unix_ms))
            .collect::<JobResult<Vec<_>>>()?;

        sqlx::query_file!(
            "queries/telemetry/record_impressions.sql",
            &event_ids,
            &request_ids,
            &user_ids,
            &track_ids,
            &cluster_ids,
            &sources,
            &positions,
            &scores_present,
            &scores,
            &features,
            &shown_at
        )
        .execute(&self.impressions)
        .await
        .map_err(JobError::retryable)?;
        Ok(())
    }

    pub async fn record_hard_negative(&self, job: &LeasedJob) -> JobResult {
        let event = payload::<HardNegative>(job)?;
        validate_hard_negative(&event)?;
        let detected_at = timestamp(event.created_at_unix_ms)?;

        let result = sqlx::query_file!(
            "queries/telemetry/record_hard_negative.sql",
            event.event_id,
            event.user_id,
            event.track_id,
            event.position_pct,
            detected_at
        )
        .fetch_one(&self.hard_negatives)
        .await
        .map_err(JobError::retryable)?;
        if result.recorded {
            return Ok(());
        }
        if let Some(wait) = impression_wait_left(detected_at, Utc::now()) {
            return Err(JobError::postponed(
                wait,
                anyhow::anyhow!("matching recommendation impression is not available yet"),
            ));
        }
        tracing::debug!(
            event_id = %event.event_id,
            "skip without a prior recommendation impression is not a hard negative"
        );
        Ok(())
    }
}

fn validate_batch(batch: &ImpressionBatch) -> JobResult {
    if batch.impressions.is_empty() || batch.impressions.len() > MAX_IMPRESSIONS {
        return invalid("impression batch size is invalid");
    }

    for item in &batch.impressions {
        if item.user_id.is_empty()
            || item.track_id.is_empty()
            || item.cluster_id.is_empty()
            || item.source.is_empty()
            || item.source.len() > 16
            || item.position < 0
            || item.score.is_some_and(|score| !score.is_finite())
            || item.features.as_ref().is_some_and(|features| {
                features.len() > MAX_FEATURES || features.iter().any(|value| !value.is_finite())
            })
        {
            return invalid("impression payload is invalid");
        }
    }
    Ok(())
}

fn validate_hard_negative(event: &HardNegative) -> JobResult {
    if event.user_id.is_empty()
        || event.track_id.is_empty()
        || !event.position_pct.is_finite()
        || !(0.0..=1.0).contains(&event.position_pct)
    {
        return invalid("hard negative payload is invalid");
    }
    Ok(())
}

fn impression_wait_left(
    detected_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<std::time::Duration> {
    (detected_at + chrono::Duration::minutes(IMPRESSION_WAIT_MINUTES) - now)
        .to_std()
        .ok()
        .filter(|left| !left.is_zero())
}

fn timestamp(milliseconds: i64) -> JobResult<DateTime<Utc>> {
    DateTime::from_timestamp_millis(milliseconds)
        .ok_or_else(|| JobError::permanent(anyhow::anyhow!("timestamp is out of range")))
}

fn invalid<T>(message: &'static str) -> JobResult<T> {
    Err(JobError::permanent(anyhow::anyhow!(message)))
}

#[cfg(test)]
mod tests {
    use backend_contracts::{Impression, Versioned};
    use uuid::Uuid;

    use super::*;

    #[test]
    fn non_finite_scores_are_rejected() {
        let batch = ImpressionBatch {
            request_id: Uuid::nil(),
            impressions: vec![Impression {
                impression_id: Uuid::nil(),
                user_id: "user".to_owned(),
                track_id: "track".to_owned(),
                cluster_id: "cluster".to_owned(),
                position: 0,
                score: Some(f32::NAN),
                features: None,
                source: "home".to_owned(),
                shown_at_unix_ms: 1,
            }],
        };

        assert!(validate_batch(&batch).is_err());
    }

    async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
        sqlx::raw_sql(include_str!(
            "../../../api/migrations-ops/9000_rec_telemetry.sql"
        ))
        .execute(pool)
        .await?;
        sqlx::raw_sql(include_str!(
            "../../../api/migrations-ops/9001_rec_telemetry_idempotency.sql"
        ))
        .execute(pool)
        .await?;
        sqlx::raw_sql(
            "CREATE UNIQUE INDEX rec_hard_negatives_event_id_idx
                 ON rec_hard_negatives (event_id) WHERE event_id IS NOT NULL",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn install_sweep_schema(pool: &PgPool) -> anyhow::Result<()> {
        install_schema(pool).await?;
        sqlx::raw_sql(include_str!(
            "../../../api/migrations-ops/9011_rec_hard_negative_sweep.sql"
        ))
        .execute(pool)
        .await?;
        sqlx::raw_sql(
            "CREATE TABLE user_events (
                 id uuid PRIMARY KEY,
                 sc_user_id text NOT NULL,
                 sc_track_id text NOT NULL,
                 event_type text NOT NULL,
                 weight double precision NOT NULL DEFAULT 0,
                 position_pct real,
                 created_at timestamp NOT NULL DEFAULT now()
             )",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn record_event(
        pool: &PgPool,
        user: &str,
        track: &str,
        event_type: &str,
        position_pct: Option<f32>,
        minutes_ago: i64,
    ) -> anyhow::Result<Uuid> {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO user_events (id, sc_user_id, sc_track_id, event_type, position_pct, created_at)
             VALUES ($1, $2, $3, $4, $5, (now() AT TIME ZONE 'UTC') - $6::bigint * interval '1 minute')",
        )
        .bind(id)
        .bind(user)
        .bind(track)
        .bind(event_type)
        .bind(position_pct)
        .bind(minutes_ago)
        .execute(pool)
        .await?;
        Ok(id)
    }

    async fn show(pool: &PgPool, user: &str, track: &str, minutes_ago: i64) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO rec_impressions (
                 sc_user_id, sc_track_id, cluster_id, source, position, score, shown_at
             ) VALUES ($1, $2, 'cluster', 'home', 0, 0.7, now() - $3::bigint * interval '1 minute')",
        )
        .bind(user)
        .bind(track)
        .bind(minutes_ago)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn hard_negative_events(pool: &PgPool) -> anyhow::Result<Vec<Uuid>> {
        Ok(
            sqlx::query_scalar("SELECT event_id FROM rec_hard_negatives ORDER BY event_id")
                .fetch_all(pool)
                .await?,
        )
    }

    fn hard_negative_job(event: HardNegative) -> LeasedJob {
        LeasedJob {
            id: Uuid::now_v7(),
            kind: backend_contracts::JobKind::RecordHardNegative,
            dedup_key: None,
            payload: serde_json::to_value(Versioned::V1(event)).unwrap(),
            generation: 1,
            attempts: 1,
            max_attempts: 8,
            lease_id: Uuid::new_v4(),
        }
    }

    #[sqlx::test(migrations = false)]
    async fn hard_negative_waits_for_a_prior_impression(pool: PgPool) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        let handler = TelemetryHandler::new(pool.clone(), pool.clone(), pool.clone());
        let detected_at = Utc::now();
        let event = HardNegative {
            event_id: Uuid::now_v7(),
            user_id: "user".to_owned(),
            track_id: "track".to_owned(),
            position_pct: 0.1,
            created_at_unix_ms: detected_at.timestamp_millis(),
        };
        let job = hard_negative_job(event.clone());

        let missing = handler.record_hard_negative(&job).await.unwrap_err();
        assert!(matches!(missing, JobError::Postponed { .. }));

        sqlx::query(
            "INSERT INTO rec_impressions (
                 sc_user_id, sc_track_id, cluster_id, source, position, score, shown_at
             ) VALUES ('user', 'track', 'cluster', 'home', 0, 0.9, $1)",
        )
        .bind(detected_at + chrono::Duration::seconds(1))
        .execute(&pool)
        .await?;
        let future_only = handler.record_hard_negative(&job).await.unwrap_err();
        assert!(matches!(future_only, JobError::Postponed { .. }));

        sqlx::query(
            "INSERT INTO rec_impressions (
                 sc_user_id, sc_track_id, cluster_id, source, position, score, shown_at
             ) VALUES ('user', 'track', 'cluster', 'home', 0, 0.4, $1)",
        )
        .bind(detected_at - chrono::Duration::seconds(1))
        .execute(&pool)
        .await?;
        handler.record_hard_negative(&job).await?;
        handler.record_hard_negative(&job).await?;

        let score: Option<f32> = sqlx::query_scalar(
            "SELECT predicted_score FROM rec_hard_negatives WHERE event_id = $1",
        )
        .bind(event.event_id)
        .fetch_one(&pool)
        .await?;
        assert_eq!(score, Some(0.4));
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn a_skip_never_recommended_finishes_quietly_after_the_wait(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        let handler = TelemetryHandler::new(pool.clone(), pool.clone(), pool.clone());
        let detected_at = Utc::now() - chrono::Duration::minutes(IMPRESSION_WAIT_MINUTES + 1);
        let job = hard_negative_job(HardNegative {
            event_id: Uuid::now_v7(),
            user_id: "user".to_owned(),
            track_id: "track".to_owned(),
            position_pct: 0.1,
            created_at_unix_ms: detected_at.timestamp_millis(),
        });

        handler.record_hard_negative(&job).await?;

        let recorded: i64 = sqlx::query_scalar("SELECT count(*) FROM rec_hard_negatives")
            .fetch_one(&pool)
            .await?;
        assert_eq!(recorded, 0);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn a_fresh_miss_is_postponed_to_the_end_of_the_window(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        let handler = TelemetryHandler::new(pool.clone(), pool.clone(), pool.clone());
        let detected_at = Utc::now() - chrono::Duration::minutes(3);
        let job = hard_negative_job(HardNegative {
            event_id: Uuid::now_v7(),
            user_id: "user".to_owned(),
            track_id: "track".to_owned(),
            position_pct: 0.1,
            created_at_unix_ms: detected_at.timestamp_millis(),
        });

        let failure = handler.record_hard_negative(&job).await.unwrap_err();

        let JobError::Postponed { delay, .. } = failure else {
            panic!("a fresh miss must wait without spending an attempt, got {failure:?}");
        };
        let left = chrono::Duration::minutes(IMPRESSION_WAIT_MINUTES - 3);
        assert!(delay <= left.to_std()?);
        assert!(delay >= (left - chrono::Duration::seconds(5)).to_std()?);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn the_sweep_records_recommended_early_skips_once_the_wait_is_over(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        install_sweep_schema(&pool).await?;
        let handler = TelemetryHandler::new(pool.clone(), pool.clone(), pool.clone());
        show(&pool, "user-a", "track-1", 30).await?;
        show(&pool, "user-a", "track-3", 30).await?;
        let recommended = record_event(&pool, "user-a", "track-1", "skip", Some(0.1), 20).await?;
        record_event(&pool, "user-b", "track-2", "skip", Some(0.1), 20).await?;
        record_event(&pool, "user-a", "track-1", "skip", Some(0.5), 20).await?;
        record_event(&pool, "user-a", "track-1", "full_play", None, 20).await?;
        let fresh = record_event(&pool, "user-a", "track-3", "skip", Some(0.05), 2).await?;

        handler.sweep_hard_negatives().await?;
        handler.sweep_hard_negatives().await?;

        assert_eq!(hard_negative_events(&pool).await?, vec![recommended]);

        sqlx::query(
            "UPDATE user_events SET created_at = created_at - interval '15 minutes' WHERE id = $1",
        )
        .bind(fresh)
        .execute(&pool)
        .await?;
        handler.sweep_hard_negatives().await?;

        let mut expected = vec![recommended, fresh];
        expected.sort();
        assert_eq!(hard_negative_events(&pool).await?, expected);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn one_sweep_walks_a_backlog_larger_than_a_batch(pool: PgPool) -> anyhow::Result<()> {
        install_sweep_schema(&pool).await?;
        let handler = TelemetryHandler::new(pool.clone(), pool.clone(), pool.clone());
        show(&pool, "user-a", "track-old", 4 * 24 * 60).await?;
        let forgotten =
            record_event(&pool, "user-a", "track-old", "skip", Some(0.1), 3 * 24 * 60).await?;
        sqlx::query(
            "INSERT INTO user_events (id, sc_user_id, sc_track_id, event_type, position_pct, created_at)
             SELECT gen_random_uuid(), 'listener', 'track-' || n, 'skip', 0.1,
                    (now() AT TIME ZONE 'UTC') - interval '2 hours' + n * interval '1 millisecond'
             FROM generate_series(1, $1::int) AS n",
        )
        .bind(i32::try_from(SWEEP_BATCH * 2 + 1)?)
        .execute(&pool)
        .await?;
        show(&pool, "user-a", "track-1", 90).await?;
        let last = record_event(&pool, "user-a", "track-1", "skip", Some(0.1), 60).await?;

        handler.sweep_hard_negatives().await?;

        assert_eq!(hard_negative_events(&pool).await?, vec![last]);
        assert!(!hard_negative_events(&pool).await?.contains(&forgotten));
        let scanned: Uuid = sqlx::query_scalar(
            "SELECT scanned_event_id FROM rec_hard_negative_sweep WHERE singleton",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(scanned, last);
        Ok(())
    }

    #[test]
    fn the_impression_wait_is_minutes_not_hours() {
        let detected_at = Utc::now();
        assert_eq!(
            impression_wait_left(
                detected_at,
                detected_at + chrono::Duration::minutes(IMPRESSION_WAIT_MINUTES - 1)
            ),
            Some(std::time::Duration::from_secs(60))
        );
        assert_eq!(
            impression_wait_left(
                detected_at,
                detected_at + chrono::Duration::minutes(IMPRESSION_WAIT_MINUTES)
            ),
            None
        );
        assert_eq!(
            impression_wait_left(detected_at, detected_at + chrono::Duration::hours(1)),
            None
        );
    }
}
