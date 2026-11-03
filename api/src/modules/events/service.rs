use std::sync::Arc;
use std::time::Duration;

use mini_moka::sync::Cache;
use sqlx::PgPool;
use tokio::sync::{Mutex as AsyncMutex, OnceCell};
use tracing::warn;
use uuid::Uuid;

use crate::background_jobs::{CollabJobs, IndexingJobs};
use crate::common::sc_ids::normalize_sc_track_id;
use crate::error::{AppError, AppResult};
use crate::modules::dislikes::DislikesService;

const LIKE_WEIGHT: f64 = 1.0;
pub const PLAYLIST_ADD_WEIGHT: f64 = 0.9;
const FULL_PLAY_WEIGHT: f64 = 0.3;
const SKIP_WEIGHT: f64 = -0.5;
const DISLIKE_WEIGHT: f64 = -1.0;

const USER_LOCK_CAPACITY: u64 = 16_384;
const USER_LOCK_TTL: Duration = Duration::from_secs(5 * 60);

const POSITIVE_EVENTS: &[&str] = &["like", "playlist_add"];
const COLLAB_TRIGGER_EVENTS: &[&str] = &["like", "playlist_add", "full_play", "skip"];

fn event_weight(event_type: &str) -> Option<f64> {
    match event_type {
        "like" => Some(LIKE_WEIGHT),
        "playlist_add" => Some(PLAYLIST_ADD_WEIGHT),
        "full_play" => Some(FULL_PLAY_WEIGHT),
        "skip" => Some(SKIP_WEIGHT),
        "dislike" => Some(DISLIKE_WEIGHT),
        _ => None,
    }
}

fn skip_weight_from_position(position_pct: Option<f32>) -> f64 {
    match position_pct {
        Some(p) if p < 0.20 => -0.8,
        Some(p) if p < 0.70 => -0.3,
        Some(_) => 0.0,
        None => SKIP_WEIGHT,
    }
}

fn validate_position_pct(position_pct: Option<f32>) -> AppResult<()> {
    if position_pct
        .is_some_and(|position| !position.is_finite() || !(0.0..=1.0).contains(&position))
    {
        return Err(AppError::bad_request(
            "positionPct must be a finite number between 0 and 1",
        ));
    }
    Ok(())
}

async fn track_needs_storage(pg: &PgPool, sc_track_id: &str) -> AppResult<bool> {
    Ok(sqlx::query_file_scalar!(
        "queries/events/service/track_needs_storage.sql",
        sc_track_id
    )
    .fetch_one(pg)
    .await?)
}

pub struct EventsService {
    pg: PgPool,
    indexing_jobs: Arc<IndexingJobs>,
    collab_jobs: Arc<CollabJobs>,
    user_locks: Cache<String, Arc<AsyncMutex<()>>>,
    dislikes: OnceCell<Arc<DislikesService>>,
}

impl EventsService {
    pub fn new(
        pg: PgPool,
        indexing_jobs: Arc<IndexingJobs>,
        collab_jobs: Arc<CollabJobs>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pg,
            indexing_jobs,
            collab_jobs,
            user_locks: Cache::builder()
                .max_capacity(USER_LOCK_CAPACITY)
                .time_to_idle(USER_LOCK_TTL)
                .build(),
            dislikes: OnceCell::new(),
        })
    }

    pub fn install_dislikes(&self, dislikes: Arc<DislikesService>) {
        let _ = self.dislikes.set(dislikes);
    }

    fn lock_for(&self, key: &str) -> Arc<AsyncMutex<()>> {
        if let Some(lock) = self.user_locks.get(&key.to_string()) {
            return lock;
        }
        let lock = Arc::new(AsyncMutex::new(()));
        self.user_locks.insert(key.to_string(), lock.clone());
        lock
    }

    async fn enqueue_indexing(&self, sc_track_id: &str) {
        let queued = match track_needs_storage(&self.pg, sc_track_id).await {
            Ok(true) => self.indexing_jobs.enqueue(sc_track_id).await,
            Ok(false) => Ok(()),
            Err(error) => Err(error),
        };
        if let Err(error) = queued {
            warn!(track = sc_track_id, %error, "indexing trigger enqueue failed");
        }
    }

    pub async fn record(
        self: &Arc<Self>,
        sc_user_id: &str,
        sc_track_id: &str,
        event_type: &str,
        position_pct: Option<f32>,
    ) -> AppResult<()> {
        validate_position_pct(position_pct)?;
        let Some(mut weight) = event_weight(event_type) else {
            warn!(event_type, "Unknown event type");
            return Ok(());
        };
        let Some(normalized) = normalize_sc_track_id(sc_track_id) else {
            warn!(sc_track_id, "Invalid scTrackId");
            return Ok(());
        };

        let is_positive = POSITIVE_EVENTS.contains(&event_type);
        if is_positive
            && let Some(d) = self.dislikes.get()
            && d.is_disliked_by_user_id(sc_user_id, &normalized)
                .await
                .unwrap_or(false)
        {
            return Ok(());
        }

        if event_type == "skip" {
            weight = skip_weight_from_position(position_pct);
        }

        let lock_key = format!("events:{sc_user_id}");
        let lock = self.lock_for(&lock_key);
        let user_guard = lock.lock().await;

        let event_id = Uuid::now_v7();
        match position_pct {
            Some(position) => {
                sqlx::query_file!(
                    "queries/events/service/insert_event_with_position.sql",
                    event_id,
                    sc_user_id,
                    &normalized,
                    event_type,
                    weight,
                    position,
                )
                .execute(&self.pg)
                .await?;
            }
            None => {
                sqlx::query_file!(
                    "queries/events/service/insert_event.sql",
                    event_id,
                    sc_user_id,
                    &normalized,
                    event_type,
                    weight,
                )
                .execute(&self.pg)
                .await?;
            }
        }
        drop(user_guard);

        self.enqueue_indexing(&normalized).await;

        if COLLAB_TRIGGER_EVENTS.contains(&event_type) {
            self.collab_jobs.note_event().await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_rejects_non_finite_values() {
        assert!(validate_position_pct(Some(f32::NAN)).is_err());
    }

    #[test]
    fn position_rejects_values_outside_fraction_range() {
        assert!(validate_position_pct(Some(1.01)).is_err());
    }

    #[test]
    fn position_accepts_fraction_boundaries() {
        assert!(
            validate_position_pct(Some(0.0)).is_ok() && validate_position_pct(Some(1.0)).is_ok()
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn only_a_track_still_waiting_for_its_audio_gets_an_index_trigger(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, storage_state)
             SELECT id, 'soundcloud:tracks:' || id, 'Song', 'song', 180000, state
             FROM (VALUES ('1', 'pending'), ('2', 'failed'), ('3', 'missing'),
                          ('4', 'ok'), ('5', 'too_long')) AS seeded (id, state)",
        )
        .execute(&pool)
        .await?;

        let mut needs_storage = Vec::new();
        for sc_track_id in ["1", "2", "3", "4", "5", "6"] {
            needs_storage.push(track_needs_storage(&pool, sc_track_id).await?);
        }
        assert_eq!(needs_storage, [true, true, true, false, false, false]);
        Ok(())
    }
}
