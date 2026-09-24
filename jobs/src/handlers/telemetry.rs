use backend_contracts::{HardNegative, ImpressionBatch};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;

use crate::queue::{JobError, JobResult, LeasedJob};

use super::payload;

const MAX_IMPRESSIONS: usize = 2_048;
const MAX_FEATURES: usize = 512;

pub struct TelemetryHandler {
    impressions: PgPool,
    hard_negatives: PgPool,
}

impl TelemetryHandler {
    pub fn new(impressions: PgPool, hard_negatives: PgPool) -> Self {
        Self {
            impressions,
            hard_negatives,
        }
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
        if !result.recorded {
            return Err(JobError::retryable(anyhow::anyhow!(
                "matching recommendation impression is not available yet"
            )));
        }
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
        let handler = TelemetryHandler::new(pool.clone(), pool.clone());
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
        assert!(missing.is_retryable());

        sqlx::query(
            "INSERT INTO rec_impressions (
                 sc_user_id, sc_track_id, cluster_id, source, position, score, shown_at
             ) VALUES ('user', 'track', 'cluster', 'home', 0, 0.9, $1)",
        )
        .bind(detected_at + chrono::Duration::seconds(1))
        .execute(&pool)
        .await?;
        let future_only = handler.record_hard_negative(&job).await.unwrap_err();
        assert!(future_only.is_retryable());

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
}
