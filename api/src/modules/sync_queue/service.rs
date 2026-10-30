use std::sync::Arc;

use deadpool_redis::Pool as RedisPool;
use serde_json::Value;
use sqlx::{PgConnection, PgPool};

use crate::error::AppResult;

use super::mirror::{self, WantedMirror};

pub struct SyncQueueService {
    pub(super) pg: PgPool,
    pub(super) redis: RedisPool,
}

impl SyncQueueService {
    pub fn new(pg: PgPool, redis: RedisPool) -> Arc<Self> {
        Arc::new(Self { pg, redis })
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
        self.set_wanted_on(
            &mut transaction,
            mirror,
            user_id,
            mirror_key,
            action_type,
            target_urn,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub(crate) async fn set_wanted_on(
        &self,
        connection: &mut PgConnection,
        mirror: WantedMirror,
        user_id: &str,
        mirror_key: &str,
        action_type: &str,
        target_urn: &str,
    ) -> AppResult<bool> {
        if !mirror::set_wanted(&mut *connection, mirror, user_id, mirror_key).await? {
            return Ok(false);
        }
        self.enqueue_on(connection, user_id, action_type, target_urn, None)
            .await?;
        Ok(true)
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
        let annuls = match inverse(action_type) {
            Some(inverse) => {
                sqlx::query_file!(
                    "queries/sync_queue/service/cancel_inverse.sql",
                    user_id,
                    inverse,
                    &target_urn
                )
                .execute(&mut *connection)
                .await?
                .rows_affected()
                    > 0
            }
            None => false,
        };

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
            .bind(annuls)
            .execute(&mut *connection)
            .await?;
        }
        Ok(())
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
