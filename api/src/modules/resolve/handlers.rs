use std::sync::Arc;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::middleware::from_fn_with_state;
use axum::response::Response;
use axum::routing::get;
use backend_contracts::CatalogEntity;
use serde::Deserialize;
use serde_json::Value;

use super::input::{EntityKey, ResolveInput};
use super::repository;
use crate::cache::cache_service::CacheScope;
use crate::common::admission::{PublicAdmission, admit_resolve};
use crate::common::response::json_response;
use crate::common::session::OptionalSession;
use crate::error::{AppError, AppResult};
use crate::modules::auth::TokenKind;
use crate::state::AppState;

pub fn router(admission: Arc<PublicAdmission>) -> Router<AppState> {
    Router::new().route(
        "/resolve",
        get(resolve).route_layer(from_fn_with_state(admission, admit_resolve)),
    )
}

#[derive(Debug, Clone, Deserialize)]
struct ResolveQuery {
    url: String,
}

async fn resolve(
    State(st): State<AppState>,
    OptionalSession(session): OptionalSession,
    Query(q): Query<ResolveQuery>,
) -> AppResult<Response> {
    let input = ResolveInput::parse(&q.url)?;
    let viewer = session.as_ref().map(|session| session.sc_user_id.as_str());
    let alias_key = st.cache.build_key(
        "GET",
        &format!("/resolve-identity-v1?url={}", input.upstream),
        CacheScope::Shared,
        None,
    );
    if !input.requires_upstream {
        let mut key = repository::find(&st.pg, &input).await?;
        let from_alias = key.is_none() && input.short_link;
        if key.is_none()
            && input.short_link
            && let Ok(Some(urn)) = st.cache.get_raw(&alias_key).await
        {
            key = EntityKey::parse(&urn);
        }
        if let Some(key) = key {
            match repository::load(&st.pg, &key, viewer, false).await {
                Ok(local) => {
                    enqueue_stale(&st, &key, &local, viewer).await;
                    return response(&local.value);
                }
                Err(error) if from_alias && error.status() == StatusCode::NOT_FOUND => {}
                Err(error) => return Err(error),
            }
        }
    }

    let token_kind = session.as_ref().map_or(TokenKind::PublicPool, |session| {
        TokenKind::UserFirst(session.session_id)
    });
    let observation = catalog_ingest::Observation::begin(&st.pg).await?;
    let fetched = match input.entity.as_ref() {
        Some(key) => match key.entity {
            CatalogEntity::Track => st.resolve.track_by_id(token_kind, &key.id).await,
            CatalogEntity::Playlist => st.resolve.playlist_meta(token_kind, &key.id).await,
            CatalogEntity::User => st.resolve.user_by_id(token_kind, &key.id).await,
            CatalogEntity::Profile | CatalogEntity::WebProfiles => {
                return Err(AppError::bad_request("Unsupported entity"));
            }
        },
        None => st.resolve.resolve(token_kind, &input.upstream).await,
    }
    .map_err(upstream_unavailable)?;
    let key = EntityKey::from_payload(&fetched)?;
    if input
        .entity
        .as_ref()
        .is_some_and(|expected| expected != &key)
    {
        return Err(AppError::coded(
            StatusCode::BAD_GATEWAY,
            "invalid_resolve_response",
            "SoundCloud returned a different entity",
        ));
    }
    persist_resolved(&st, &key, &fetched, observation).await?;
    let local = repository::load(
        &st.pg,
        &key,
        viewer,
        input.requires_upstream || input.short_link,
    )
    .await?;
    if input.short_link && !input.requires_upstream && local.public {
        let _ = st
            .cache
            .set_raw(
                &alias_key,
                &key.urn(),
                86400,
                None,
                CacheScope::Shared,
                None,
            )
            .await;
    }
    response(&local.value)
}

const UPSTREAM_RETRY_AFTER: i64 = 30;

pub(super) fn upstream_unavailable(error: AppError) -> AppError {
    if matches!(error, AppError::ScUnreachable(_)) {
        return AppError::coded(
            StatusCode::BAD_GATEWAY,
            "resolve_upstream_unavailable",
            "SoundCloud could not be reached for this link",
        )
        .with_retry_after(UPSTREAM_RETRY_AFTER);
    }
    error
}

fn response(value: &Value) -> AppResult<Response> {
    let payload = serde_json::to_string(value)
        .map_err(|error| AppError::internal(format!("json encode: {error}")))?;
    let mut response = json_response(StatusCode::OK, payload);
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store, private"),
    );
    Ok(response)
}

async fn persist_resolved(
    st: &AppState,
    key: &EntityKey,
    value: &Value,
    observation: catalog_ingest::Observation,
) -> AppResult<()> {
    match key.entity {
        CatalogEntity::Track => {
            st.indexing
                .ingest_track_from_sc(value, catalog_ingest::TrackPriority::Discovery, observation)
                .await?
        }
        CatalogEntity::User => {
            crate::modules::users::UserRepository::new(st.pg.clone())
                .upsert_from_sc(value, observation)
                .await?;
        }
        CatalogEntity::Playlist => {
            crate::modules::playlists::PlaylistRepository::new(st.pg.clone())
                .upsert_from_sc(value, observation)
                .await?;
        }
        CatalogEntity::Profile | CatalogEntity::WebProfiles => {
            return Err(AppError::bad_request("Unsupported entity"));
        }
    }
    Ok(())
}

async fn enqueue_stale(
    st: &AppState,
    key: &EntityKey,
    local: &repository::LocalEntity,
    viewer: Option<&str>,
) {
    let ttl = match key.entity {
        CatalogEntity::Track => st.config.cold.track_ttl_sec,
        CatalogEntity::Playlist => st.config.cold.playlist_ttl_sec,
        CatalogEntity::User => st.config.cold.user_ttl_sec,
        CatalogEntity::Profile | CatalogEntity::WebProfiles => return,
    };
    let age = chrono::Utc::now()
        .signed_duration_since(local.synced_at)
        .num_seconds();
    if age >= 0 && age as u64 <= ttl {
        return;
    }
    let owner = if local.public {
        None
    } else {
        local
            .owner
            .as_deref()
            .filter(|owner| Some(*owner) == viewer)
    };
    if !local.public && owner.is_none() {
        return;
    }
    if let Err(error) =
        crate::modules::cold_refresh::entity::enqueue_entity(&st.pg, key.entity, &key.urn(), owner)
            .await
    {
        tracing::debug!(%error, "resolved entity refresh enqueue deferred");
    }
}
