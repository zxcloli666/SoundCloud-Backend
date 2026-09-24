use std::sync::Arc;

use serde_json::{Value, json};
use sqlx::PgPool;

use crate::error::AppResult;
use crate::modules::cold_refresh::{ColdRefreshService, FOLLOWINGS};
use crate::modules::sync_queue::SyncQueueService;
use crate::modules::sync_queue::mirror::FOLLOWINGS as FOLLOWINGS_MIRROR;
pub struct MeService {
    pg: PgPool,
    sync_queue: Arc<SyncQueueService>,
    cold_refresh: Arc<ColdRefreshService>,
}

impl MeService {
    pub fn new(
        pg: PgPool,
        sync_queue: Arc<SyncQueueService>,
        cold_refresh: Arc<ColdRefreshService>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pg,
            sync_queue,
            cold_refresh,
        })
    }

    pub async fn get_profile(&self, sc_user_id: &str) -> AppResult<Value> {
        super::profile::read(&self.pg, sc_user_id).await
    }

    pub async fn get_followings_tracks(
        &self,
        sc_user_id: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<super::FollowingsTracksPage> {
        let sync = self
            .cold_refresh
            .ensure_collection(FOLLOWINGS, sc_user_id, true)
            .await?;
        let page = super::feed::read(&self.pg, sc_user_id, page, limit).await?;
        Ok(super::FollowingsTracksPage {
            page,
            followings_sync: sync,
        })
    }

    pub async fn follow_user(&self, sc_user_id: &str, target_user_urn: &str) -> AppResult<Value> {
        let target_user_urn = super::feed::target_urn(target_user_urn)?;
        let target_user_urn = target_user_urn.as_str();
        let mut tx = self.pg.begin().await?;
        crate::modules::sync_queue::mirror::set_wanted(
            &mut tx,
            FOLLOWINGS_MIRROR,
            sc_user_id,
            target_user_urn,
        )
        .await?;
        self.sync_queue
            .enqueue_on(&mut tx, sc_user_id, "follow_user", target_user_urn, None)
            .await?;
        sqlx::query_file!(
            "queries/me/service/enqueue_followed_uploads.sql",
            &[crate::common::sc_ids::extract_sc_id(target_user_urn).to_owned()],
            crate::common::sc_ids::extract_sc_id(sc_user_id)
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(json!({ "status": "queued", "actionType": "follow_user" }))
    }

    pub async fn unfollow_user(&self, sc_user_id: &str, target_user_urn: &str) -> AppResult<Value> {
        let target_user_urn = super::feed::target_urn(target_user_urn)?;
        let target_user_urn = target_user_urn.as_str();
        self.sync_queue
            .clear_wanted(
                FOLLOWINGS_MIRROR,
                sc_user_id,
                target_user_urn,
                "unfollow_user",
                target_user_urn,
            )
            .await?;
        Ok(json!({ "status": "queued", "actionType": "unfollow_user" }))
    }
}

pub fn premium_response(premium: bool) -> Value {
    json!({ "premium": premium })
}
