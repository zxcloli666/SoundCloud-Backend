use std::fmt::{Display, Formatter};

use backend_contracts::JobKind;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct NewJob {
    pub id: Uuid,
    pub kind: JobKind,
    pub dedup_key: Option<String>,
    pub payload: Value,
    pub priority: i16,
    pub max_attempts: i16,
    pub available_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaimOrder {
    Priority,
    Oldest,
}

#[derive(Clone, Debug)]
pub struct LeasedJob {
    pub id: Uuid,
    pub kind: JobKind,
    pub dedup_key: Option<String>,
    pub payload: Value,
    pub generation: i64,
    pub attempts: i32,
    pub max_attempts: i16,
    pub lease_id: Uuid,
}

#[derive(Debug, FromRow)]
pub(super) struct LeasedJobRow {
    pub id: Uuid,
    pub kind: String,
    pub dedup_key: Option<String>,
    pub payload: Value,
    pub generation: i64,
    pub attempts: i32,
    pub max_attempts: i16,
    pub lease_id: Uuid,
}

impl TryFrom<LeasedJobRow> for LeasedJob {
    type Error = QueueError;

    fn try_from(row: LeasedJobRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            kind: row.kind.parse().map_err(QueueError::UnknownKind)?,
            dedup_key: row.dedup_key,
            payload: row.payload,
            generation: row.generation,
            attempts: row.attempts,
            max_attempts: row.max_attempts,
            lease_id: row.lease_id,
        })
    }
}

#[derive(Debug)]
pub enum JobError {
    Retryable(anyhow::Error),
    Permanent(anyhow::Error),
    Postponed {
        delay: std::time::Duration,
        error: anyhow::Error,
    },
}

impl JobError {
    pub fn postponed(delay: std::time::Duration, error: impl Into<anyhow::Error>) -> Self {
        Self::Postponed {
            delay: delay.clamp(
                std::time::Duration::from_secs(1),
                std::time::Duration::from_secs(86400),
            ),
            error: error.into(),
        }
    }

    pub fn retryable(error: impl Into<anyhow::Error>) -> Self {
        Self::Retryable(error.into())
    }

    pub fn permanent(error: impl Into<anyhow::Error>) -> Self {
        Self::Permanent(error.into())
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable(_))
    }
}

impl Display for JobError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Retryable(error) | Self::Permanent(error) | Self::Postponed { error, .. } => {
                Display::fmt(error, formatter)
            }
        }
    }
}

impl std::error::Error for JobError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Retryable(error) | Self::Permanent(error) | Self::Postponed { error, .. } => {
                error.source()
            }
        }
    }
}

pub type JobResult<T = ()> = Result<T, JobError>;

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("database operation failed: {0}")]
    Database(#[from] sqlx::Error),

    #[error("{0}")]
    UnknownKind(#[from] backend_contracts::UnknownJobKind),

    #[error("job batch is too large")]
    BatchTooLarge,

    #[error("all job kinds in a claim must belong to the same lane")]
    MixedLanes,

    #[error("job deduplication key must contain between 1 and 1024 bytes")]
    InvalidDedupKey,
}
