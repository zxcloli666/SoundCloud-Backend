use std::time::Duration;

use backend_contracts::{JobKind, JobLane};

use super::{JobRepository, duration_milliseconds};
use crate::queue::model::{ClaimOrder, LeasedJob, LeasedJobRow, QueueError};

impl JobRepository {
    pub async fn claim(
        &self,
        supported_kinds: &[JobKind],
        order: ClaimOrder,
        limit: usize,
        lease_duration: Duration,
    ) -> Result<Vec<LeasedJob>, QueueError> {
        if supported_kinds.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let lane = single_lane(supported_kinds)?;
        let limit = i64::try_from(limit).map_err(|_| QueueError::BatchTooLarge)?;
        let lease_milliseconds = duration_milliseconds(lease_duration)?;
        let supported_kinds = supported_kinds
            .iter()
            .map(|kind| kind.as_str().to_owned())
            .collect::<Vec<_>>();

        let rows = match order {
            ClaimOrder::Priority => {
                sqlx::query_file_as!(
                    LeasedJobRow,
                    "queries/queue/claim_priority.sql",
                    limit,
                    &supported_kinds,
                    self.worker_id.as_str(),
                    lease_milliseconds,
                    lane.as_str()
                )
                .fetch_all(&self.pool)
                .await?
            }
            ClaimOrder::Kind(kind) => {
                sqlx::query_file_as!(
                    LeasedJobRow,
                    "queries/queue/claim_kind.sql",
                    limit,
                    kind.as_str(),
                    self.worker_id.as_str(),
                    lease_milliseconds
                )
                .fetch_all(&self.pool)
                .await?
            }
            ClaimOrder::Oldest => {
                sqlx::query_file_as!(
                    LeasedJobRow,
                    "queries/queue/claim_oldest.sql",
                    limit,
                    &supported_kinds,
                    self.worker_id.as_str(),
                    lease_milliseconds,
                    lane.as_str()
                )
                .fetch_all(&self.pool)
                .await?
            }
        };
        rows.into_iter()
            .map(LeasedJob::try_from)
            .collect::<Result<Vec<_>, _>>()
    }

    pub async fn release_expired(
        &self,
        supported_kinds: &[JobKind],
        limit: usize,
    ) -> Result<u64, QueueError> {
        if supported_kinds.is_empty() || limit == 0 {
            return Ok(0);
        }

        let lane = single_lane(supported_kinds)?;
        let limit = i64::try_from(limit).map_err(|_| QueueError::BatchTooLarge)?;
        let supported_kinds = supported_kinds
            .iter()
            .map(|kind| kind.as_str().to_owned())
            .collect::<Vec<_>>();
        let result = sqlx::query_file!(
            "queries/queue/release_expired.sql",
            limit,
            &supported_kinds,
            lane.as_str()
        )
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }
}

fn single_lane(supported_kinds: &[JobKind]) -> Result<JobLane, QueueError> {
    let lane = supported_kinds[0].lane();
    if supported_kinds.iter().any(|kind| kind.lane() != lane) {
        return Err(QueueError::MixedLanes);
    }
    Ok(lane)
}
