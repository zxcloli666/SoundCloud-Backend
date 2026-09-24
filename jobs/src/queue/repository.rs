mod claim;
mod completion;
mod enqueue;
mod recovery;

#[cfg(test)]
mod tests;

use std::time::Duration;

use sqlx::PgPool;

use super::model::QueueError;

const MAX_DEDUP_KEY_LENGTH: usize = 1_024;

#[derive(Clone)]
pub struct JobRepository {
    pool: PgPool,
    worker_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Completion {
    Completed,
    Superseded,
    LostLease,
}

impl JobRepository {
    pub fn new(pool: PgPool, worker_id: String) -> Self {
        Self { pool, worker_id }
    }
}

fn duration_milliseconds(duration: Duration) -> Result<i64, QueueError> {
    i64::try_from(duration.as_millis()).map_err(|_| QueueError::BatchTooLarge)
}

fn validate_dedup_key(dedup_key: Option<&str>) -> Result<(), QueueError> {
    match dedup_key {
        Some(value) if value.is_empty() || value.len() > MAX_DEDUP_KEY_LENGTH => {
            Err(QueueError::InvalidDedupKey)
        }
        _ => Ok(()),
    }
}

fn truncate(value: &str, maximum: usize) -> String {
    match value.char_indices().nth(maximum) {
        Some((index, _)) => value[..index].to_owned(),
        None => value.to_owned(),
    }
}
