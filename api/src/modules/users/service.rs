use crate::modules::cold_refresh::collection::CollectionPage;
use std::sync::Arc;

use serde_json::Value;
use sqlx::PgPool;

use crate::common::sc_ids::extract_sc_id;
use crate::error::{AppError, AppResult};
use crate::modules::cold_refresh::{
    ColdRefreshService, FOLLOWERS, FOLLOWINGS, LIKED_PLAYLISTS, LIKED_TRACKS, OWNED_PLAYLISTS,
    OWNED_TRACKS, read_collection_page,
};
use crate::modules::likes::cold as likes_cold;

pub struct UsersService {
    pg: PgPool,
    cold_refresh: Arc<ColdRefreshService>,
}

impl UsersService {
    pub fn new(pg: PgPool, cold_refresh: Arc<ColdRefreshService>) -> Arc<Self> {
        Arc::new(Self { pg, cold_refresh })
    }

    pub async fn get_by_id(&self, user_urn: &str) -> AppResult<Value> {
        let repo = crate::modules::users::UserRepository::new(self.pg.clone());
        if let Some(row) = repo.find_by_urn(user_urn).await? {
            let synced_at = row.sc_synced_at;
            let _ = repo.touch_last_read(user_urn).await;
            if self.cold_refresh.is_user_stale(Some(synced_at))
                && let Err(error) = crate::modules::cold_refresh::entity::enqueue_entity(
                    &self.pg,
                    backend_contracts::CatalogEntity::User,
                    user_urn,
                    None,
                )
                .await
            {
                tracing::debug!(%error, "catalog refresh enqueue deferred");
            }
            return Ok(crate::modules::users::project_to_sc_shape(&row));
        }
        Err(crate::modules::cold_refresh::entity::refresh_pending(
            &self.pg,
            backend_contracts::CatalogEntity::User,
            user_urn,
            None,
            "user_refresh_pending",
            "Profile is being loaded",
        )
        .await)
    }

    pub async fn get_followers(
        &self,
        user_urn: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<CollectionPage> {
        let sync = self
            .cold_refresh
            .ensure_collection(FOLLOWERS, user_urn, false)
            .await?;
        let page = read_collection_page(&self.pg, &FOLLOWERS, user_urn, page, limit, true).await?;
        Ok(CollectionPage::new(page, sync))
    }

    pub async fn get_is_following(
        &self,
        viewer_sc_user_id: &str,
        user_urn: &str,
        following_urn: &str,
    ) -> AppResult<bool> {
        let owner = same_sc_user(viewer_sc_user_id, user_urn);
        self.cold_refresh
            .ensure_collection(FOLLOWINGS, user_urn, owner)
            .await?;
        let mirrored = sqlx::query_file_scalar!(
            "queries/users/service/is_following.sql",
            extract_sc_id(user_urn),
            &crate::common::sc_ids::user_id_variants(user_urn),
            following_urn,
            if owner { "owner" } else { "public" }
        )
        .fetch_one(&self.pg)
        .await?;
        mirrored.ok_or_else(|| {
            AppError::coded(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "collection_refresh_pending",
                "Followings are being loaded",
            )
            .with_retry_after(5)
        })
    }

    pub async fn get_owned_tracks(
        &self,
        viewer_sc_user_id: &str,
        target_sc_user_id: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<CollectionPage> {
        let is_self = same_sc_user(viewer_sc_user_id, target_sc_user_id);
        let sync = self
            .cold_refresh
            .ensure_collection(OWNED_TRACKS, target_sc_user_id, is_self)
            .await?;
        let mut result = read_collection_page(
            &self.pg,
            &OWNED_TRACKS,
            target_sc_user_id,
            page,
            limit,
            !is_self,
        )
        .await?;
        likes_cold::apply_user_favorite_flag(&self.pg, viewer_sc_user_id, &mut result.collection)
            .await?;
        Ok(CollectionPage::new(result, sync))
    }

    pub async fn get_owned_playlists(
        &self,
        viewer_sc_user_id: &str,
        target_sc_user_id: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<CollectionPage> {
        let is_self = same_sc_user(viewer_sc_user_id, target_sc_user_id);
        let sync = self
            .cold_refresh
            .ensure_collection(OWNED_PLAYLISTS, target_sc_user_id, is_self)
            .await?;
        let mut result = read_collection_page(
            &self.pg,
            &OWNED_PLAYLISTS,
            target_sc_user_id,
            page,
            limit,
            !is_self,
        )
        .await?;
        likes_cold::apply_user_favorite_flag_to_playlists(
            &self.pg,
            viewer_sc_user_id,
            &mut result.collection,
        )
        .await?;
        Ok(CollectionPage::new(result, sync))
    }

    pub async fn get_liked_tracks(
        self: &Arc<Self>,
        viewer_sc_user_id: &str,
        target_sc_user_id: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<CollectionPage> {
        let is_self = same_sc_user(viewer_sc_user_id, target_sc_user_id);
        let sync = self
            .cold_refresh
            .ensure_collection(LIKED_TRACKS, target_sc_user_id, is_self)
            .await?;
        let mut result = read_collection_page(
            &self.pg,
            &LIKED_TRACKS,
            target_sc_user_id,
            page,
            limit,
            !is_self,
        )
        .await?;

        if is_self {
            for t in result.collection.iter_mut() {
                if let Some(obj) = t.as_object_mut() {
                    obj.insert("user_favorite".into(), Value::Bool(true));
                }
            }
        } else {
            likes_cold::apply_user_favorite_flag(
                &self.pg,
                viewer_sc_user_id,
                &mut result.collection,
            )
            .await?;
        }
        Ok(CollectionPage::new(result, sync))
    }

    pub async fn get_liked_playlists(
        &self,
        viewer_sc_user_id: &str,
        target_sc_user_id: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<CollectionPage> {
        let is_self = same_sc_user(viewer_sc_user_id, target_sc_user_id);
        let sync = self
            .cold_refresh
            .ensure_collection(LIKED_PLAYLISTS, target_sc_user_id, is_self)
            .await?;
        let mut result = read_collection_page(
            &self.pg,
            &LIKED_PLAYLISTS,
            target_sc_user_id,
            page,
            limit,
            !is_self,
        )
        .await?;
        likes_cold::apply_user_favorite_flag_to_playlists(
            &self.pg,
            viewer_sc_user_id,
            &mut result.collection,
        )
        .await?;
        Ok(CollectionPage::new(result, sync))
    }

    pub async fn get_followings(
        &self,
        viewer_sc_user_id: &str,
        target_sc_user_id: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<CollectionPage> {
        let is_self = same_sc_user(viewer_sc_user_id, target_sc_user_id);
        let sync = self
            .cold_refresh
            .ensure_collection(FOLLOWINGS, target_sc_user_id, is_self)
            .await?;
        let result = read_collection_page(
            &self.pg,
            &FOLLOWINGS,
            target_sc_user_id,
            page,
            limit,
            !is_self,
        )
        .await?;
        Ok(CollectionPage::new(result, sync))
    }

    pub async fn get_web_profiles(&self, user_urn: &str) -> AppResult<Value> {
        super::web_profiles::read(&self.pg, user_urn).await
    }
}

fn same_sc_user(viewer: &str, target: &str) -> bool {
    crate::common::sc_ids::extract_sc_id(viewer) == crate::common::sc_ids::extract_sc_id(target)
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
        sqlx::raw_sql(
            "CREATE TABLE user_followings (
                 user_id text NOT NULL,
                 target_user_urn text NOT NULL,
                 wanted_state boolean NOT NULL DEFAULT true,
                 PRIMARY KEY (user_id, target_user_urn)
             );
             CREATE TABLE catalog_collection_sync (
                 subject_id text NOT NULL,
                 collection text NOT NULL,
                 scope text NOT NULL,
                 synced_at timestamptz,
                 PRIMARY KEY (subject_id, collection, scope)
             );",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn mirrored(pool: &PgPool, follower: &str, target: &str) -> anyhow::Result<Option<bool>> {
        Ok(sqlx::query_file_scalar!(
            "queries/users/service/is_following.sql",
            crate::common::sc_ids::extract_sc_id(follower),
            &crate::common::sc_ids::user_id_variants(follower),
            target,
            "owner"
        )
        .fetch_one(pool)
        .await?)
    }

    #[sqlx::test(migrations = false)]
    async fn an_unsynced_collection_answers_only_known_local_relations(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::query(
            "INSERT INTO user_followings (user_id, target_user_urn)
             VALUES ('42', 'soundcloud:users:7')",
        )
        .execute(&pool)
        .await?;

        assert_eq!(
            mirrored(&pool, "42", "soundcloud:users:7").await?,
            Some(true)
        );
        assert_eq!(mirrored(&pool, "42", "soundcloud:users:8").await?, None);
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn a_synced_collection_answers_both_ways_without_soundcloud(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        install_schema(&pool).await?;
        sqlx::query(
            "INSERT INTO catalog_collection_sync (subject_id, collection, scope, synced_at)
             VALUES ('42', 'followings', 'owner', now())",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "INSERT INTO user_followings (user_id, target_user_urn, wanted_state)
             VALUES ('soundcloud:users:42', 'soundcloud:users:7', true),
                    ('soundcloud:users:42', 'soundcloud:users:8', false)",
        )
        .execute(&pool)
        .await?;

        assert_eq!(
            mirrored(&pool, "soundcloud:users:42", "soundcloud:users:7").await?,
            Some(true)
        );
        assert_eq!(
            mirrored(&pool, "soundcloud:users:42", "soundcloud:users:8").await?,
            Some(false)
        );
        assert_eq!(
            mirrored(&pool, "soundcloud:users:42", "soundcloud:users:9").await?,
            Some(false)
        );
        Ok(())
    }
}
