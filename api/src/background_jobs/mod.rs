mod collab;
mod indexing;

use std::sync::Arc;
use std::time::Duration;

use backend_contracts::{JOB_INGRESS_SUBJECT, JobCommand, JobKind, Versioned};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::bus::nats::NatsService;
use crate::error::{AppError, AppResult};

pub use collab::CollabJobs;
pub use indexing::IndexingJobs;

const DEFAULT_MAX_ATTEMPTS: i16 = 8;
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(2);
const OPPORTUNISTIC_PUBLISH_TIMEOUT: Duration = Duration::from_millis(150);
const TELEMETRY_PUBLISH_TIMEOUT: Duration = Duration::from_millis(150);

pub struct BackgroundJobs {
    nats: Arc<NatsService>,
}

pub struct BackgroundJob {
    id: Uuid,
    kind: JobKind,
    dedup_key: Option<String>,
    enqueue_if_absent: bool,
    payload: Value,
    priority: i16,
    max_attempts: i16,
    available_at: DateTime<Utc>,
}

impl BackgroundJobs {
    pub fn new(nats: Arc<NatsService>) -> Arc<Self> {
        Arc::new(Self { nats })
    }

    pub async fn enqueue(&self, job: &BackgroundJob) -> AppResult<Uuid> {
        self.publish(job, PUBLISH_TIMEOUT).await
    }

    pub async fn enqueue_telemetry(&self, job: &BackgroundJob) -> AppResult<Uuid> {
        self.publish(job, TELEMETRY_PUBLISH_TIMEOUT).await
    }

    pub async fn enqueue_opportunistic(&self, job: &BackgroundJob) -> bool {
        match self.publish(job, OPPORTUNISTIC_PUBLISH_TIMEOUT).await {
            Ok(_) => true,
            Err(error) => {
                tracing::debug!(%error, kind = %job.kind, "background job enqueue skipped");
                false
            }
        }
    }

    async fn publish(&self, job: &BackgroundJob, timeout: Duration) -> AppResult<Uuid> {
        let command = Versioned::V1(job.command());
        let message_id = job.id.to_string();
        tokio::time::timeout(
            timeout,
            self.nats
                .publish_dedup(JOB_INGRESS_SUBJECT, &command, &message_id),
        )
        .await
        .map_err(|_| AppError::internal("background job ingress timed out"))??;
        Ok(job.id)
    }
}

impl BackgroundJob {
    pub fn unique<T>(kind: JobKind, payload: T) -> AppResult<Self>
    where
        T: Serialize,
    {
        Self::new(kind, None, payload)
    }

    pub fn coalescing<T>(kind: JobKind, dedup_key: impl Into<String>, payload: T) -> AppResult<Self>
    where
        T: Serialize,
    {
        Self::new(kind, Some(dedup_key.into()), payload)
    }

    pub fn with_priority(mut self, priority: i16) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    pub fn if_absent(mut self) -> Self {
        self.enqueue_if_absent = true;
        self
    }

    pub fn with_max_attempts(mut self, max_attempts: i16) -> AppResult<Self> {
        if max_attempts <= 0 {
            return Err(AppError::internal(
                "background job max_attempts must be positive",
            ));
        }
        self.max_attempts = max_attempts;
        Ok(self)
    }

    fn new<T>(kind: JobKind, dedup_key: Option<String>, payload: T) -> AppResult<Self>
    where
        T: Serialize,
    {
        let payload = serde_json::to_value(Versioned::V1(payload)).map_err(|error| {
            AppError::internal(format!("background job serialization failed: {error}"))
        })?;

        Ok(Self {
            id: Uuid::now_v7(),
            kind,
            dedup_key,
            enqueue_if_absent: false,
            payload,
            priority: 0,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            available_at: Utc::now(),
        })
    }

    fn command(&self) -> JobCommand {
        JobCommand {
            id: self.id,
            kind: self.kind,
            dedup_key: self.dedup_key.clone(),
            enqueue_if_absent: self.enqueue_if_absent,
            payload: self.payload.clone(),
            priority: self.priority,
            max_attempts: self.max_attempts,
            available_at_unix_ms: self.available_at.timestamp_millis(),
        }
    }
}

#[cfg(test)]
mod tests {
    use backend_contracts::EmptyPayload;

    use super::*;

    #[test]
    fn payload_is_versioned_at_the_boundary() {
        let job = BackgroundJob::unique(JobKind::DiscoverAggregates, EmptyPayload {});

        assert!(matches!(
            job,
            Ok(BackgroundJob { payload, .. })
                if payload == serde_json::json!({ "version": "1", "payload": {} })
        ));
    }

    #[test]
    fn invalid_attempt_limit_is_rejected() {
        let job = BackgroundJob::unique(JobKind::DiscoverAggregates, EmptyPayload {})
            .and_then(|job| job.with_max_attempts(0));

        assert!(job.is_err());
    }

    #[test]
    fn command_preserves_routing_and_delivery_fields() {
        let job =
            BackgroundJob::coalescing(JobKind::DiscoverAggregates, "summary", EmptyPayload {})
                .unwrap()
                .with_priority(4);
        let command = job.command();

        assert_eq!(command.id, job.id);
        assert_eq!(command.kind, JobKind::DiscoverAggregates);
        assert_eq!(command.dedup_key.as_deref(), Some("summary"));
        assert!(!command.enqueue_if_absent);
        assert_eq!(command.priority, 4);
        assert_eq!(command.max_attempts, DEFAULT_MAX_ATTEMPTS);
    }

    #[test]
    fn if_absent_delivery_is_explicit() {
        let job =
            BackgroundJob::coalescing(JobKind::DiscoverAggregates, "summary", EmptyPayload {})
                .unwrap()
                .if_absent();

        assert!(job.command().enqueue_if_absent);
    }
}
