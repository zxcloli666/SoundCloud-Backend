use std::sync::Arc;
use std::time::Duration;

use deadpool_redis::Pool as RedisPool;
use deadpool_redis::redis::AsyncCommands;
use serde_json::Value;
use sqlx::{PgConnection, PgPool};

use crate::error::AppResult;

use super::mirror::{self, WantedMirror};

const COUNTS_CACHE_TTL_SECONDS: u64 = 5;
const REDIS_TIMEOUT: Duration = Duration::from_millis(150);

#[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncCounts {
    pub pending_count: i64,
    pub failed_count: i64,
    pub delayed_count: i64,
    pub retry_in_sec: Option<i64>,
}

pub struct SyncQueueService {
    pg: PgPool,
    redis: RedisPool,
}

impl SyncQueueService {
    pub fn new(pg: PgPool, redis: RedisPool) -> Arc<Self> {
        Arc::new(Self { pg, redis })
    }

    pub async fn status_for_user(&self, sc_user_id: &str) -> AppResult<SyncCounts> {
        let sc_user_id = crate::common::sc_ids::extract_sc_id(sc_user_id);
        let key = format!("sync_queue:status:{sc_user_id}");
        if let Some(counts) = self.cached_counts(&key).await {
            return Ok(counts);
        }

        let variants = crate::common::sc_ids::user_id_variants(sc_user_id);
        let row = sqlx::query_file!("queries/sync_queue/service/pending_counts.sql", &variants)
            .fetch_one(&self.pg)
            .await?;
        let counts = SyncCounts {
            pending_count: row.pending,
            failed_count: row.failed,
            delayed_count: row.delayed,
            retry_in_sec: row.retry_in_sec,
        };
        self.cache_counts(&key, &counts).await;
        Ok(counts)
    }

    pub async fn enqueue(
        &self,
        user_id: &str,
        action_type: &str,
        target_urn: &str,
        payload: Option<&Value>,
    ) -> AppResult<()> {
        let mut transaction = self.pg.begin().await?;
        self.enqueue_on(&mut transaction, user_id, action_type, target_urn, payload)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn set_wanted(
        &self,
        mirror: WantedMirror,
        user_id: &str,
        mirror_key: &str,
        action_type: &str,
        target_urn: &str,
    ) -> AppResult<()> {
        let mut transaction = self.pg.begin().await?;
        mirror::set_wanted(&mut transaction, mirror, user_id, mirror_key).await?;
        self.enqueue_on(&mut transaction, user_id, action_type, target_urn, None)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn clear_wanted(
        &self,
        mirror: WantedMirror,
        user_id: &str,
        mirror_key: &str,
        action_type: &str,
        target_urn: &str,
    ) -> AppResult<()> {
        let mut transaction = self.pg.begin().await?;
        mirror::clear_wanted(&mut transaction, mirror, user_id, mirror_key).await?;
        self.enqueue_on(&mut transaction, user_id, action_type, target_urn, None)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn enqueue_on(
        &self,
        connection: &mut PgConnection,
        user_id: &str,
        action_type: &str,
        target_urn: &str,
        payload: Option<&Value>,
    ) -> AppResult<()> {
        let user_id = crate::common::sc_ids::extract_sc_id(user_id);
        let target_urn = canonical_target(action_type, target_urn);
        if let Some(inverse) = inverse(action_type) {
            let cancelled = sqlx::query_file!(
                "queries/sync_queue/service/cancel_inverse.sql",
                user_id,
                inverse,
                &target_urn
            )
            .execute(&mut *connection)
            .await?;
            if cancelled.rows_affected() > 0 {
                return Ok(());
            }
        }

        if action_type == "comment" {
            sqlx::query(
                "INSERT INTO sync_queue (user_id, action_type, target_urn, payload)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(user_id)
            .bind(action_type)
            .bind(&target_urn)
            .bind(payload)
            .execute(&mut *connection)
            .await?;
        } else {
            sqlx::query(include_str!(
                "../../../queries/sync_queue/service/enqueue.sql"
            ))
            .bind(user_id)
            .bind(action_type)
            .bind(&target_urn)
            .bind(payload)
            .execute(&mut *connection)
            .await?;
        }
        Ok(())
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
}

fn inverse(action_type: &str) -> Option<&'static str> {
    match action_type {
        "like_track" => Some("unlike_track"),
        "unlike_track" => Some("like_track"),
        "like_playlist" => Some("unlike_playlist"),
        "unlike_playlist" => Some("like_playlist"),
        "follow_user" => Some("unfollow_user"),
        "unfollow_user" => Some("follow_user"),
        _ => None,
    }
}

fn canonical_target(action_type: &str, target: &str) -> String {
    let entity = crate::common::sc_ids::extract_sc_id(target);
    match action_type {
        "like_track" | "unlike_track" | "track_update" | "track_delete" | "comment" => {
            format!("soundcloud:tracks:{entity}")
        }
        "like_playlist" | "unlike_playlist" | "playlist_delete" | "playlist_update" => {
            format!("soundcloud:playlists:{entity}")
        }
        "follow_user" | "unfollow_user" => format!("soundcloud:users:{entity}"),
        _ => target.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn sync_status_is_local_and_scoped_to_the_session_account(
        pg: PgPool,
    ) -> anyhow::Result<()> {
        let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
        let service = SyncQueueService::new(pg.clone(), redis);
        service
            .enqueue(
                "42",
                "track_update",
                "1",
                Some(&serde_json::json!({"track": {"title": "name"}})),
            )
            .await?;
        service
            .enqueue("soundcloud:users:42", "track_delete", "2", None)
            .await?;
        service.enqueue("99", "track_delete", "3", None).await?;
        sqlx::query("UPDATE sync_queue SET retry_count = 2 WHERE user_id = '42' AND action_type = 'track_delete'")
            .execute(&pg).await?;
        let status = service.status_for_user("soundcloud:users:42").await?;
        assert_eq!(
            serde_json::to_value(&status)?,
            serde_json::json!({
                "pendingCount": 1,
                "failedCount": 1,
                "delayedCount": 0,
                "retryInSec": null
            })
        );
        assert_eq!(service.status_for_user("42").await?, status);
        assert_eq!(
            service.status_for_user("99").await?,
            SyncCounts {
                pending_count: 1,
                failed_count: 0,
                delayed_count: 0,
                retry_in_sec: None
            }
        );
        assert_eq!(
            service.status_for_user("100").await?,
            SyncCounts {
                pending_count: 0,
                failed_count: 0,
                delayed_count: 0,
                retry_in_sec: None
            }
        );
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn postponed_writes_report_when_soundcloud_will_be_retried(
        pg: PgPool,
    ) -> anyhow::Result<()> {
        let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
            .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
        let service = SyncQueueService::new(pg.clone(), redis);
        service.enqueue("7", "like_track", "1", None).await?;
        service.enqueue("7", "like_track", "2", None).await?;
        service.enqueue("7", "like_track", "3", None).await?;
        sqlx::query(
            "UPDATE sync_queue SET last_error = 'banned', next_run_at = now() + interval '20 minutes'
             WHERE user_id = '7' AND target_urn LIKE '%:1'",
        )
        .execute(&pg)
        .await?;
        sqlx::query(
            "UPDATE sync_queue SET retry_count = 1, last_error = 'boom', next_run_at = now() + interval '90 seconds'
             WHERE user_id = '7' AND target_urn LIKE '%:2'",
        )
        .execute(&pg)
        .await?;

        let status = service.status_for_user("7").await?;

        assert_eq!(status.pending_count, 2);
        assert_eq!(status.failed_count, 1);
        assert_eq!(status.delayed_count, 1);
        assert!(matches!(status.retry_in_sec, Some(85..=90)));
        Ok(())
    }

    #[test]
    fn cached_status_round_trips_and_rejects_the_old_format() {
        let counts = SyncCounts {
            pending_count: 3,
            failed_count: 1,
            delayed_count: 2,
            retry_in_sec: Some(600),
        };
        let cached = serde_json::to_string(&counts).unwrap();
        assert_eq!(serde_json::from_str::<SyncCounts>(&cached).unwrap(), counts);
        assert!(serde_json::from_str::<SyncCounts>("12:3").is_err());
    }

    #[test]
    fn only_reversible_state_actions_have_an_inverse() {
        assert_eq!(inverse("like_track"), Some("unlike_track"));
        assert_eq!(inverse("comment"), None);
    }

    #[test]
    fn queue_targets_have_one_canonical_identity() {
        assert_eq!(
            canonical_target("like_track", "42"),
            canonical_target("like_track", "soundcloud:tracks:42")
        );
        assert_eq!(canonical_target("playlist_create", "new:42"), "new:42");
    }
}
