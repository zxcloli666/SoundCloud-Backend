use backend_contracts::{JobKind, Versioned};
use serde::Serialize;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;
use wt::{Enqueued, EnqueuedJob, JobEnqueue, ObserveClaim};

use crate::error::{AppError, AppResult};

const MAX_ATTEMPTS: i16 = 8;

pub fn job<T: Serialize>(
    kind: JobKind,
    dedup_key: String,
    payload: T,
    priority: i16,
) -> AppResult<EnqueuedJob> {
    let payload = serde_json::to_value(Versioned::V1(payload))
        .map_err(|error| AppError::internal(error.to_string()))?;
    Ok(EnqueuedJob {
        id: Uuid::now_v7(),
        kind: kind.as_str().to_owned(),
        lane: kind.lane().as_str().to_owned(),
        dedup_key,
        payload,
        priority,
        max_attempts: MAX_ATTEMPTS,
        raise_priority: false,
        star_candidates: Vec::new(),
        star_bonus: 0,
    })
}

pub async fn enqueue_in(
    connection: &mut PgConnection,
    jobs: Vec<EnqueuedJob>,
    claim: Option<ObserveClaim>,
) -> AppResult<Option<Enqueued>> {
    Ok(wt::call(connection, &JobEnqueue { jobs, claim }).await?)
}

pub async fn enqueue(
    pool: &PgPool,
    jobs: Vec<EnqueuedJob>,
    claim: Option<ObserveClaim>,
) -> AppResult<Option<Enqueued>> {
    let command = JobEnqueue { jobs, claim };
    Ok(wt::retry(|| async {
        let mut connection = pool.acquire().await?;
        wt::call(&mut connection, &command).await
    })
    .await?)
}
