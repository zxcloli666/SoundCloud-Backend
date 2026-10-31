#[cfg(test)]
#[path = "vectors_tests.rs"]
mod tests;

use std::collections::HashMap;
use std::future::Future;

use backend_contracts::vector_store::TRACKS_TASTE_DIMENSIONS;
use futures::TryStreamExt;
use sqlx::PgPool;

use crate::qdrant::QdrantProvisioner;
use crate::queue::{JobError, JobResult};

use super::dataset::EventRow;
use super::history::{HistoryGrouper, UserHistory};
use super::pooling::Pooling;
use super::recent::{self, RecentHistory};

const STORE_BATCH: usize = 1_000;
const LOOKUP_BATCH: usize = 1_000;
const RECENT_EVENTS_PER_WINDOW: usize = 4;

pub(super) type UserVectors = Vec<(String, Vec<f32>)>;
pub(super) type Items = HashMap<u64, Vec<f32>>;

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
    let mut rows = sqlx::query_file_as!(
        EventRow,
        "queries/taste/export_events.sql",
        days,
        backend_contracts::SUPERLIKE_WEIGHT
    )
    .fetch(pool);
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

pub(super) struct PooledUsers {
    pub(super) vectors: UserVectors,
    pub(super) full_reads: usize,
}

pub(super) trait ItemSource: Sync {
    fn vectors_of(&self, tracks: &[u64]) -> impl Future<Output = JobResult<Items>> + Send;
}

pub(super) struct Collection<'a> {
    pub(super) qdrant: &'a QdrantProvisioner,
    pub(super) name: &'a str,
}

impl ItemSource for Collection<'_> {
    async fn vectors_of(&self, tracks: &[u64]) -> JobResult<Items> {
        let mut items = Items::with_capacity(tracks.len());
        for chunk in tracks.chunks(LOOKUP_BATCH) {
            let found = self
                .qdrant
                .retrieve_vectors(self.name, chunk)
                .await
                .map_err(JobError::retryable)?;
            items.extend(
                found
                    .into_iter()
                    .filter_map(|(id, vector)| Some((id.parse::<u64>().ok()?, vector))),
            );
        }
        Ok(items)
    }
}

pub(super) async fn pool_users(
    pool: &PgPool,
    source: &impl ItemSource,
    history_days: u32,
    pooling: &Pooling,
    users: &[String],
    now_unix: i64,
) -> JobResult<PooledUsers> {
    let dimensions = dimensions()?;
    let days = i32::try_from(history_days).map_err(JobError::permanent)?;
    let per_kind = i64::try_from(pooling.window().saturating_mul(RECENT_EVENTS_PER_WINDOW))
        .map_err(JobError::permanent)?;
    let recent = recent::histories(pool, days, &variants_of(users), per_kind).await?;
    let (whole, cut): (Vec<RecentHistory>, Vec<RecentHistory>) = recent
        .into_iter()
        .partition(|recent| recent.cutoff_unix_s.is_none());
    let (cut, short): (Vec<RecentHistory>, Vec<RecentHistory>) = cut
        .into_iter()
        .partition(|recent| pooling.serves(recent.history.positives()));
    let mut histories: Vec<UserHistory> = whole
        .into_iter()
        .map(|recent| recent.history)
        .filter(|history| pooling.serves(history.positives()))
        .collect();
    let asked = tracks_of(
        histories
            .iter()
            .chain(cut.iter().map(|recent| &recent.history)),
    );
    let mut items = source.vectors_of(&asked).await?;
    let mut unsure: Vec<String> = short
        .into_iter()
        .map(|recent| recent.history.user_id)
        .collect();
    for recent in cut {
        let item = |track: u64| items.get(&track).map(Vec::as_slice);
        let whole_window = recent.cutoff_unix_s.is_some_and(|cutoff| {
            pooling.holds_window_after(&recent.history.events, item, cutoff, dimensions)
        });
        if whole_window {
            histories.push(recent.history);
        } else {
            unsure.push(recent.history.user_id);
        }
    }
    let full_reads = unsure.len();
    if !unsure.is_empty() {
        let full = full_histories(pool, days, pooling, &unsure).await?;
        let missing: Vec<u64> = tracks_of(full.iter())
            .into_iter()
            .filter(|track| asked.binary_search(track).is_err())
            .collect();
        items.extend(source.vectors_of(&missing).await?);
        histories.extend(full);
    }
    Ok(PooledUsers {
        vectors: pool_known(histories, pooling, &items, now_unix, dimensions),
        full_reads,
    })
}

async fn full_histories(
    pool: &PgPool,
    days: i32,
    pooling: &Pooling,
    users: &[String],
) -> JobResult<Vec<UserHistory>> {
    let rows = sqlx::query_file_as!(
        EventRow,
        "queries/taste/user_events.sql",
        days,
        &variants_of(users),
        backend_contracts::SUPERLIKE_WEIGHT
    )
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
    Ok(histories)
}

fn pool_known(
    histories: Vec<UserHistory>,
    pooling: &Pooling,
    items: &Items,
    now_unix: i64,
    dimensions: usize,
) -> UserVectors {
    histories
        .into_iter()
        .filter_map(|history| {
            let item = |track: u64| items.get(&track).map(Vec::as_slice);
            let vector = pooling.user_vector(&history.events, item, now_unix, dimensions)?;
            Some((history.user_id, vector))
        })
        .collect()
}

fn tracks_of<'a>(histories: impl Iterator<Item = &'a UserHistory>) -> Vec<u64> {
    let mut tracks: Vec<u64> = histories
        .flat_map(|history| history.events.iter().map(|event| event.track))
        .collect();
    tracks.sort_unstable();
    tracks.dedup();
    tracks
}

fn variants_of(users: &[String]) -> Vec<String> {
    users
        .iter()
        .flat_map(|user| [user.clone(), format!("soundcloud:users:{user}")])
        .collect()
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
