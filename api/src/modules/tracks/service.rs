use std::sync::Arc;

use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common::sc_ids::extract_sc_id;
use crate::error::AppResult;
use crate::modules::auth::{TokenKind, TokenProvider, try_with_chain};
use crate::modules::cold_refresh::collection::CollectionPage;
use crate::modules::cold_refresh::{
    AudienceCollection, ColdRefreshService, TRACK_FAVORITERS, TRACK_REPOSTERS,
};
use crate::modules::likes::cold as likes_cold;
use crate::modules::sync_queue::SyncQueueService;
use crate::sc::ScClient;

pub struct TracksService {
    sc: ScClient,
    pg: PgPool,
    sync_queue: Arc<SyncQueueService>,
    cold_refresh: Arc<ColdRefreshService>,
    tokens: Arc<TokenProvider>,
    mutations: super::mutations::TrackMutations,
}

pub(crate) struct TracksServiceDependencies {
    pub sc: ScClient,
    pub pg: PgPool,
    pub sync_queue: Arc<SyncQueueService>,
    pub cold_refresh: Arc<ColdRefreshService>,
    pub tokens: Arc<TokenProvider>,
}

impl TracksService {
    pub(crate) fn new(dependencies: TracksServiceDependencies) -> Arc<Self> {
        let TracksServiceDependencies {
            sc,
            pg,
            sync_queue,
            cold_refresh,
            tokens,
        } = dependencies;
        let mutations = super::mutations::TrackMutations::new(pg.clone(), sync_queue.clone());
        Arc::new(Self {
            sc,
            pg,
            sync_queue,
            cold_refresh,
            tokens,
            mutations,
        })
    }

    pub async fn get_by_id(
        &self,
        session_id: uuid::Uuid,
        sc_user_id: &str,
        track_urn: &str,
        params: &[(String, String)],
    ) -> AppResult<Value> {
        let has_secret = params.iter().any(|(key, _)| key == "secret_token");
        self.get_by_id_with_fetch(sc_user_id, track_urn, has_secret, || async {
            let chain = self.tokens.chain(TokenKind::UserFirst(session_id)).await?;
            try_with_chain(&chain, |token| {
                let sc = self.sc.clone();
                let path = format!("/tracks/{track_urn}");
                let params = params.to_vec();
                async move { sc.api_get_value(&path, &token, Some(&params)).await }
            })
            .await
        })
        .await
    }

    pub(crate) async fn get_by_id_with_fetch<F, Fut>(
        &self,
        sc_user_id: &str,
        track_urn: &str,
        has_secret: bool,
        fetch: F,
    ) -> AppResult<Value>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = AppResult<Value>>,
    {
        let canonical = super::mutations::target_urn(track_urn)?;
        let track_urn = canonical.as_str();
        self.ensure_read_access(sc_user_id, track_urn, has_secret)
            .await?;
        let sc_track_id = extract_sc_id(track_urn);
        let row = sqlx::query_file_as!(
            crate::modules::tracks::TrackRow,
            "queries/tracks/service/find_by_sc_track_id.sql",
            sc_track_id
        )
        .fetch_optional(&self.pg)
        .await?;
        if row.as_ref().is_some_and(|row| row.deleted_at.is_some()) {
            return Err(crate::error::AppError::not_found("Track not found"));
        }
        let known_locally = row.is_some();
        let local = row.filter(|row| {
            row.sharing == "public"
                || row
                    .uploader_sc_user_id
                    .as_deref()
                    .is_some_and(|owner| extract_sc_id(owner) == extract_sc_id(sc_user_id))
        });
        let verified_secret = if let Some(row) = local {
            let _ = sqlx::query_file!("queries/tracks/service/touch_last_read.sql", sc_track_id)
                .execute(&self.pg)
                .await;
            if self.cold_refresh.is_track_stale(Some(row.sc_synced_at))
                && let Err(error) = crate::modules::cold_refresh::entity::enqueue_entity(
                    &self.pg,
                    backend_contracts::CatalogEntity::Track,
                    track_urn,
                    (row.sharing != "public").then_some(sc_user_id),
                )
                .await
            {
                tracing::debug!(%error, "catalog refresh enqueue deferred");
            }
            false
        } else if !has_secret {
            if known_locally {
                return Err(crate::error::AppError::not_found("Track not found"));
            }
            return Err(crate::modules::cold_refresh::entity::refresh_pending(
                &self.pg,
                backend_contracts::CatalogEntity::Track,
                track_urn,
                None,
                "track_refresh_pending",
                "Track is being loaded",
            )
            .await);
        } else {
            let indexing = self
                .cold_refresh
                .indexing_for_ingest()
                .ok_or_else(|| crate::error::AppError::internal("Catalog ingest is unavailable"))?;
            let observation = catalog_ingest::Observation::begin(&self.pg).await?;
            let fetched = fetch().await?;
            crate::common::sc_payload::validate_entity_identity(
                &fetched,
                backend_contracts::CatalogEntity::Track,
                sc_track_id,
            )?;
            indexing
                .ingest_track_from_sc(
                    &fetched,
                    crate::modules::tracks::TrackPriority::Discovery,
                    observation,
                )
                .await?;
            has_secret
        };
        let track = crate::modules::tracks::project_many(&self.pg, &[sc_track_id.to_owned()])
            .await?
            .into_iter()
            .flatten()
            .next()
            .ok_or_else(|| crate::error::AppError::not_found("Track not found"))?;
        self.ensure_read_access(sc_user_id, track_urn, verified_secret)
            .await?;
        let mut single = vec![track];
        likes_cold::apply_user_favorite_flag(&self.pg, sc_user_id, &mut single).await?;
        single
            .into_iter()
            .next()
            .ok_or_else(|| crate::error::AppError::not_found("Track not found"))
    }

    pub async fn update(
        &self,
        sc_user_id: &str,
        track_urn: &str,
        body: &Value,
    ) -> AppResult<Value> {
        self.mutations.update(sc_user_id, track_urn, body).await
    }

    pub async fn set_sharing(
        &self,
        sc_user_id: &str,
        track_urn: &str,
        sharing: &str,
    ) -> AppResult<Value> {
        self.mutations
            .update(
                sc_user_id,
                track_urn,
                &json!({"track": {"sharing": sharing}}),
            )
            .await
    }

    pub async fn delete(&self, sc_user_id: &str, track_urn: &str) -> AppResult<Value> {
        self.mutations.delete(sc_user_id, track_urn).await
    }

    pub async fn ensure_read_access(
        &self,
        sc_user_id: &str,
        track_urn: &str,
        has_secret: bool,
    ) -> AppResult<i64> {
        let access = sqlx::query_file!(
            "queries/tracks/service/read_access.sql",
            extract_sc_id(track_urn),
            extract_sc_id(sc_user_id)
        )
        .fetch_optional(&self.pg)
        .await?;
        let Some(access) = access else {
            return Ok(0);
        };
        if access.deleted || (access.can_read != Some(true) && !(has_secret && access.secret_ready))
        {
            return Err(crate::error::AppError::not_found("Track not found"));
        }
        Ok(access.sc_mutation_observation)
    }

    pub async fn get_comments(
        &self,
        sc_user_id: &str,
        track_urn: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<CollectionPage> {
        let sc_track_id = self.readable_sc_track_id(sc_user_id, track_urn).await?;
        self.cold_refresh
            .comments_page(&sc_track_id, page, limit)
            .await
    }

    pub async fn create_comment(
        &self,
        sc_user_id: &str,
        track_urn: &str,
        body: &Value,
    ) -> AppResult<Value> {
        let sc_track_id = self.readable_sc_track_id(sc_user_id, track_urn).await?;
        let comment = super::comments::submitted(body)?;
        let target = super::mutations::target_urn(track_urn)?;
        let mut transaction = self.pg.begin().await?;
        self.sync_queue
            .enqueue_on(&mut transaction, sc_user_id, "comment", &target, Some(body))
            .await?;
        super::comments::record_pending(&mut transaction, &sc_track_id, sc_user_id, &comment)
            .await?;
        transaction.commit().await?;
        Ok(json!({ "status": "queued", "actionType": "comment", "targetUrn": target }))
    }

    pub async fn get_favoriters(
        &self,
        sc_user_id: &str,
        track_urn: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<CollectionPage> {
        self.audience(TRACK_FAVORITERS, sc_user_id, track_urn, page, limit)
            .await
    }

    pub async fn get_reposters(
        &self,
        sc_user_id: &str,
        track_urn: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<CollectionPage> {
        self.audience(TRACK_REPOSTERS, sc_user_id, track_urn, page, limit)
            .await
    }

    async fn audience(
        &self,
        coll: AudienceCollection,
        sc_user_id: &str,
        track_urn: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<CollectionPage> {
        let track_urn = self.readable_sc_track_id(sc_user_id, track_urn).await?;
        self.cold_refresh
            .audience_page(coll, &track_urn, page, limit)
            .await
    }

    pub async fn readable_sc_track_id(
        &self,
        sc_user_id: &str,
        track_urn: &str,
    ) -> AppResult<String> {
        let canonical = super::mutations::target_urn(track_urn)?;
        self.ensure_read_access(sc_user_id, &canonical, false)
            .await?;
        Ok(extract_sc_id(&canonical).to_owned())
    }
}
