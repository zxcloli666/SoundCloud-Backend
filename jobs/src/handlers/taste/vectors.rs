use std::collections::HashMap;

use backend_contracts::vector_store::TRACKS_TASTE_DIMENSIONS;
use futures::TryStreamExt;
use sqlx::PgPool;

use crate::qdrant::QdrantProvisioner;
use crate::queue::{JobError, JobResult};

use super::dataset::EventRow;
use super::history::{HistoryGrouper, UserHistory};
use super::pooling::Pooling;

const STORE_BATCH: usize = 1_000;
const LOOKUP_BATCH: usize = 1_000;

pub(super) type UserVectors = Vec<(String, Vec<f32>)>;

pub(super) fn dimensions() -> JobResult<usize> {
    usize::try_from(TRACKS_TASTE_DIMENSIONS).map_err(JobError::permanent)
}

pub(super) async fn pool_everyone(
    pool: &PgPool,
    history_days: u32,
    pooling: &Pooling,
    items: &HashMap<u64, Vec<f32>>,
    now_unix: i64,
) -> JobResult<UserVectors> {
    let dimensions = dimensions()?;
    let days = i32::try_from(history_days).map_err(JobError::permanent)?;
    let mut vectors = Vec::new();
    let mut grouper = HistoryGrouper::default();
    let mut rows =
        sqlx::query_file_as!(EventRow, "queries/taste/export_events.sql", days).fetch(pool);
    let mut pool_one = |history: UserHistory| {
        if !pooling.serves(history.positives()) {
            return;
        }
        let item = |track: u64| items.get(&track).map(Vec::as_slice);
        if let Some(vector) = pooling.user_vector(&history.events, item, now_unix, dimensions) {
            vectors.push((history.user_id, vector));
        }
    };
    while let Some(row) = rows.try_next().await.map_err(JobError::retryable)? {
        let Some((user_id, event)) = row.into_event() else {
            continue;
        };
        if let Some(history) = grouper.push(&user_id, event) {
            pool_one(history);
        }
    }
    drop(rows);
    if let Some(history) = grouper.finish() {
        pool_one(history);
    }
    Ok(vectors)
}

pub(super) async fn pool_users(
    pool: &PgPool,
    qdrant: &QdrantProvisioner,
    history_days: u32,
    pooling: &Pooling,
    collection: &str,
    users: &[String],
    now_unix: i64,
) -> JobResult<UserVectors> {
    let dimensions = dimensions()?;
    let days = i32::try_from(history_days).map_err(JobError::permanent)?;
    let variants: Vec<String> = users
        .iter()
        .flat_map(|user| [user.clone(), format!("soundcloud:users:{user}")])
        .collect();
    let rows = sqlx::query_file_as!(EventRow, "queries/taste/user_events.sql", days, &variants)
        .fetch_all(pool)
        .await
        .map_err(JobError::retryable)?;
    let mut histories = Vec::new();
    let mut grouper = HistoryGrouper::default();
    for row in rows {
        let Some((user_id, event)) = row.into_event() else {
            continue;
        };
        if let Some(history) = grouper.push(&user_id, event) {
            histories.push(history);
        }
    }
    histories.extend(grouper.finish());
    histories.retain(|history| pooling.serves(history.positives()));

    let mut tracks: Vec<u64> = histories
        .iter()
        .flat_map(|history| history.events.iter().map(|event| event.track))
        .collect();
    tracks.sort_unstable();
    tracks.dedup();
    let mut items: HashMap<u64, Vec<f32>> = HashMap::with_capacity(tracks.len());
    for chunk in tracks.chunks(LOOKUP_BATCH) {
        let found = qdrant
            .retrieve_vectors(collection, chunk)
            .await
            .map_err(JobError::retryable)?;
        items.extend(
            found
                .into_iter()
                .filter_map(|(id, vector)| Some((id.parse::<u64>().ok()?, vector))),
        );
    }
    Ok(histories
        .into_iter()
        .filter_map(|history| {
            let item = |track: u64| items.get(&track).map(Vec::as_slice);
            let vector = pooling.user_vector(&history.events, item, now_unix, dimensions)?;
            Some((history.user_id, vector))
        })
        .collect())
}

pub(super) async fn store(
    pool: &PgPool,
    version: &str,
    vectors: &[(String, Vec<f32>)],
) -> JobResult {
    let dimensions = dimensions()?;
    let width = i32::try_from(dimensions).map_err(JobError::permanent)?;
    for batch in vectors.chunks(STORE_BATCH) {
        let users: Vec<String> = batch.iter().map(|(user, _)| user.clone()).collect();
        let flat: Vec<f32> = batch
            .iter()
            .flat_map(|(_, vector)| vector.iter().copied())
            .collect();
        if flat.len() != users.len() * dimensions {
            return Err(JobError::permanent(anyhow::anyhow!(
                "taste user vectors must have {dimensions} values each"
            )));
        }
        sqlx::query_file!(
            "queries/taste/store_vectors.sql",
            &users,
            version,
            &flat,
            width
        )
        .execute(pool)
        .await
        .map_err(JobError::retryable)?;
    }
    Ok(())
}
