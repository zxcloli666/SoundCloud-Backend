use std::time::Duration;

use deadpool_redis::redis::AsyncCommands;

use crate::error::AppResult;

use super::service::SyncQueueService;

const COUNTS_CACHE_TTL_SECONDS: u64 = 5;
const REDIS_TIMEOUT: Duration = Duration::from_millis(150);
const LISTED_PLAYLIST_CREATES: usize = 100;

#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncCounts {
    pub pending_count: i64,
    pub failed_count: i64,
    pub delayed_count: i64,
    pub retry_in_sec: Option<i64>,
    pub dead_count: i64,
    pub needs_relogin: bool,
    pub playlist_creates: Vec<PlaylistCreate>,
    pub playlist_creates_truncated: bool,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistCreate {
    pub target_urn: String,
    pub title: String,
    pub state: PlaylistCreateState,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PlaylistCreateState {
    Queued,
    Delayed,
    Failed,
}

impl SyncQueueService {
    pub async fn status_for_user(&self, sc_user_id: &str) -> AppResult<SyncCounts> {
        let sc_user_id = crate::common::sc_ids::extract_sc_id(sc_user_id);
        let key = status_key(sc_user_id);
        if let Some(counts) = self.cached_counts(&key).await {
            return Ok(counts);
        }

        let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
        let row = sqlx::query_file!("queries/sync_queue/service/pending_counts.sql", &variants)
            .fetch_one(&self.pg)
            .await?;
        let mut creates =
            sqlx::query_file!("queries/sync_queue/service/playlist_creates.sql", &variants)
                .fetch_all(&self.pg)
                .await?;
        let playlist_creates_truncated = creates.len() > LISTED_PLAYLIST_CREATES;
        creates.truncate(LISTED_PLAYLIST_CREATES);
        let counts = SyncCounts {
            pending_count: row.pending,
            failed_count: row.failed,
            delayed_count: row.delayed,
            retry_in_sec: row.retry_in_sec,
            dead_count: row.dead,
            needs_relogin: row.needs_relogin,
            playlist_creates: creates
                .into_iter()
                .map(|create| PlaylistCreate {
                    target_urn: create.target_urn,
                    title: create.title,
                    state: if create.dead {
                        PlaylistCreateState::Failed
                    } else if create.delayed {
                        PlaylistCreateState::Delayed
                    } else {
                        PlaylistCreateState::Queued
                    },
                })
                .collect(),
            playlist_creates_truncated,
        };
        self.cache_counts(&key, &counts).await;
        Ok(counts)
    }

    pub async fn pending_and_failed(&self, sc_user_id: &str) -> AppResult<(i64, i64)> {
        let sc_user_id = crate::common::sc_ids::extract_sc_id(sc_user_id);
        let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
        let row = sqlx::query_file!("queries/sync_queue/service/pending_counts.sql", &variants)
            .fetch_one(&self.pg)
            .await?;
        Ok((row.pending, row.failed))
    }

    pub async fn dead_memberships(
        &self,
        sc_user_id: &str,
        target_urn: Option<&str>,
    ) -> AppResult<Vec<String>> {
        let sc_user_id = crate::common::sc_ids::extract_sc_id(sc_user_id);
        let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
        Ok(sqlx::query_file_scalar!(
            "queries/sync_queue/service/dead_memberships.sql",
            &variants,
            target_urn
        )
        .fetch_all(&self.pg)
        .await?)
    }

    pub async fn revive(
        &self,
        sc_user_id: &str,
        target_urn: Option<&str>,
        reread_memberships: &[String],
    ) -> AppResult<i64> {
        let sc_user_id = crate::common::sc_ids::extract_sc_id(sc_user_id);
        let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
        let revived = sqlx::query_file_scalar!(
            "queries/sync_queue/service/revive_dead.sql",
            &variants,
            target_urn,
            reread_memberships
        )
        .fetch_one(&self.pg)
        .await?;
        self.forget_counts(&status_key(sc_user_id)).await;
        Ok(revived)
    }

    async fn cached_counts(&self, key: &str) -> Option<SyncCounts> {
        tokio::time::timeout(REDIS_TIMEOUT, async {
            let mut connection = self.redis.get().await.ok()?;
            let value: String = connection.get(key).await.ok()?;
            serde_json::from_str(&value).ok()
        })
        .await
        .ok()
        .flatten()
    }

    async fn cache_counts(&self, key: &str, counts: &SyncCounts) {
        let Ok(payload) = serde_json::to_string(counts) else {
            return;
        };
        let _ = tokio::time::timeout(REDIS_TIMEOUT, async {
            let mut connection = self.redis.get().await?;
            connection
                .set_ex::<_, _, ()>(key, payload, COUNTS_CACHE_TTL_SECONDS)
                .await
                .map_err(deadpool_redis::PoolError::Backend)
        })
        .await;
    }

    async fn forget_counts(&self, key: &str) {
        let _ = tokio::time::timeout(REDIS_TIMEOUT, async {
            let mut connection = self.redis.get().await?;
            connection
                .del::<_, ()>(key)
                .await
                .map_err(deadpool_redis::PoolError::Backend)
        })
        .await;
    }
}

fn status_key(sc_user_id: &str) -> String {
    format!("sync_queue:status:v2:{sc_user_id}")
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
