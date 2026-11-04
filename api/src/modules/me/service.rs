use std::sync::Arc;

use backend_contracts::{CatalogCollection, CatalogCollectionPayload, JobKind};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use crate::background_jobs::queue;
use crate::error::AppResult;
use crate::modules::cold_refresh::{ColdRefreshService, FOLLOWINGS};
use crate::modules::subscriptions::service::PremiumStatus;
use crate::modules::sync_queue::SyncQueueService;
use crate::modules::sync_queue::mirror::FOLLOWINGS as FOLLOWINGS_MIRROR;
use crate::modules::users::{PublicUsers, UserRepository};
pub struct MeService {
    pg: PgPool,
    sync_queue: Arc<SyncQueueService>,
    cold_refresh: Arc<ColdRefreshService>,
    public: Arc<PublicUsers>,
}

impl MeService {
    pub fn new(
        pg: PgPool,
        sync_queue: Arc<SyncQueueService>,
        cold_refresh: Arc<ColdRefreshService>,
        public: Arc<PublicUsers>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pg,
            sync_queue,
            cold_refresh,
            public,
        })
    }

    pub async fn get_profile(&self, session: Uuid, sc_user_id: &str) -> AppResult<Value> {
        if let Some(user) = UserRepository::new(self.pg.clone())
            .find_by_urn(sc_user_id)
            .await?
        {
            self.public.refresh(session, sc_user_id, &user).await;
        }
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
        crate::db::retry_conflicts(|| async move {
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
            let due = sqlx::query_file_scalar!(
                "queries/me/service/followed_uploads_due.sql",
                &[crate::common::sc_ids::extract_sc_id(target_user_urn).to_owned()],
                crate::common::sc_ids::extract_sc_id(sc_user_id)
            )
            .fetch_all(&mut *tx)
            .await?;
            let jobs = due
                .into_iter()
                .map(owned_tracks_job)
                .collect::<AppResult<Vec<_>>>()?;
            if !jobs.is_empty() {
                queue::enqueue_in(&mut tx, jobs, None).await?;
            }
            tx.commit().await?;
            Ok(())
        })
        .await?;
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

fn owned_tracks_job(subject_id: String) -> AppResult<wt::EnqueuedJob> {
    let payload = CatalogCollectionPayload {
        collection: CatalogCollection::OwnedTracks,
        subject_id,
        owner: false,
    };
    queue::job(JobKind::CatalogCollection, payload.dedup_key(), payload, 5)
}

pub fn premium_response(status: PremiumStatus) -> Value {
    json!({ "premium": status.premium, "starSince": status.star_since })
}
