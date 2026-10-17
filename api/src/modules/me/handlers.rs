use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::common::pagination::PaginationQuery;
use crate::common::sc_ids::EntityRef;
use crate::common::session::SessionCtx;
use crate::error::{AppError, AppResult};
use crate::modules::cold_refresh::collection::CollectionPage;
use crate::modules::enrich::dto as enrich_dto;
use crate::modules::me::service::premium_response;
use crate::modules::sync_queue::status::SyncCounts;
use crate::state::AppState;

const MAX_RETRY_TARGET_LEN: usize = 128;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RetrySyncBody {
    target_urn: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me", get(get_profile))
        .route("/me/sync", get(get_sync_status))
        .route("/me/sync/retry", post(retry_sync))
        .route("/me/subscription", get(get_subscription))
        .route("/me/likes/tracks", get(get_liked_tracks))
        .route("/me/likes/playlists", get(get_liked_playlists))
        .route("/me/followings", get(get_followings))
        .route("/me/followings/tracks", get(get_followings_tracks))
        .route(
            "/me/followings/{user_urn}",
            put(follow_user).delete(unfollow_user),
        )
        .route("/me/followers", get(get_followers))
        .route("/me/playlists", get(get_playlists))
        .route("/me/tracks", get(get_tracks))
}

async fn get_profile(State(st): State<AppState>, ctx: SessionCtx) -> AppResult<Json<Value>> {
    Ok(Json(st.me.get_profile(&ctx.sc_user_id).await?))
}

async fn get_sync_status(
    State(st): State<AppState>,
    ctx: SessionCtx,
) -> AppResult<Json<SyncCounts>> {
    Ok(Json(st.sync_queue.status_for_user(&ctx.sc_user_id).await?))
}

async fn retry_sync(
    State(st): State<AppState>,
    ctx: SessionCtx,
    body: Bytes,
) -> AppResult<Json<Value>> {
    let target_urn = retry_target(&body)?;
    let memberships = st
        .sync_queue
        .dead_memberships(&ctx.sc_user_id, target_urn.as_deref())
        .await?;
    for playlist_urn in &memberships {
        if !st.playlists.enqueue_edit_observation(playlist_urn).await {
            return Err(AppError::internal("playlist re-read was not queued"));
        }
    }
    let revived = st
        .sync_queue
        .revive(&ctx.sc_user_id, target_urn.as_deref(), &memberships)
        .await?;
    Ok(Json(json!({ "revived": revived })))
}

fn retry_target(body: &[u8]) -> AppResult<Option<String>> {
    if body.trim_ascii().is_empty() {
        return Ok(None);
    }
    let Ok(RetrySyncBody { target_urn }) = serde_json::from_slice(body) else {
        return Err(AppError::bad_request("invalid sync retry body"));
    };
    let valid = target_urn.as_deref().is_none_or(|target| {
        target.len() <= MAX_RETRY_TARGET_LEN
            && match target.strip_prefix("new:") {
                Some(nonce) => uuid::Uuid::parse_str(nonce).is_ok(),
                None => EntityRef::parse_urn(target).is_some_and(|entity| entity.urn() == target),
            }
    });
    if !valid {
        return Err(AppError::bad_request("invalid sync retry target"));
    }
    Ok(target_urn)
}

async fn get_subscription(State(st): State<AppState>, ctx: SessionCtx) -> AppResult<Json<Value>> {
    let premium = st.subscriptions.is_premium(&ctx.sc_user_id).await?;
    Ok(Json(premium_response(premium)))
}

async fn get_liked_tracks(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Query(q): Query<PaginationQuery>,
) -> AppResult<Json<CollectionPage>> {
    let (page, limit) = q.resolved();
    let mut result = st
        .users
        .get_liked_tracks(&ctx.sc_user_id, &ctx.sc_user_id, page, limit)
        .await?;
    enrich_dto::apply_to_tracks(&st.pg, &mut result.collection).await?;
    Ok(Json(result))
}

async fn get_liked_playlists(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Query(q): Query<PaginationQuery>,
) -> AppResult<Json<CollectionPage>> {
    let (page, limit) = q.resolved();
    Ok(Json(
        st.users
            .get_liked_playlists(&ctx.sc_user_id, &ctx.sc_user_id, page, limit)
            .await?,
    ))
}

async fn get_followings(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Query(q): Query<PaginationQuery>,
) -> AppResult<Json<CollectionPage>> {
    let (page, limit) = q.resolved();
    Ok(Json(
        st.users
            .get_followings(&ctx.sc_user_id, &ctx.sc_user_id, page, limit)
            .await?,
    ))
}

async fn get_followings_tracks(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Query(q): Query<PaginationQuery>,
) -> AppResult<Json<crate::modules::me::FollowingsTracksPage>> {
    let (page, limit) = q.resolved();
    let mut result = st
        .me
        .get_followings_tracks(&ctx.sc_user_id, page, limit)
        .await?;
    enrich_dto::apply_to_tracks(&st.pg, &mut result.page.collection).await?;
    Ok(Json(result))
}

async fn follow_user(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(user_urn): Path<String>,
) -> AppResult<Json<Value>> {
    let v = st.me.follow_user(&ctx.sc_user_id, &user_urn).await?;
    Ok(Json(v))
}

async fn unfollow_user(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(user_urn): Path<String>,
) -> AppResult<Json<Value>> {
    let v = st.me.unfollow_user(&ctx.sc_user_id, &user_urn).await?;
    Ok(Json(v))
}

async fn get_followers(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Query(q): Query<PaginationQuery>,
) -> AppResult<Json<CollectionPage>> {
    let (page, limit) = q.resolved();
    Ok(Json(
        st.users.get_followers(&ctx.sc_user_id, page, limit).await?,
    ))
}

async fn get_playlists(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Query(q): Query<PaginationQuery>,
) -> AppResult<Json<CollectionPage>> {
    let (page, limit) = q.resolved();
    Ok(Json(
        st.users
            .get_owned_playlists(&ctx.sc_user_id, &ctx.sc_user_id, page, limit)
            .await?,
    ))
}

async fn get_tracks(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Query(q): Query<PaginationQuery>,
) -> AppResult<Json<CollectionPage>> {
    let (page, limit) = q.resolved();
    let mut result = st
        .users
        .get_owned_tracks(&ctx.sc_user_id, &ctx.sc_user_id, page, limit)
        .await?;
    enrich_dto::apply_to_tracks(&st.pg, &mut result.collection).await?;
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::retry_target;

    fn target(body: &str) -> Option<Option<String>> {
        retry_target(body.as_bytes()).ok()
    }

    #[test]
    fn a_retry_target_is_a_queued_create_or_a_canonical_urn() {
        let create = "new:7d4c4c56-8c0f-4c2e-9a37-1b0e8a4f3e21";
        assert_eq!(
            target(&format!(r#"{{"targetUrn":"{create}"}}"#)),
            Some(Some(create.to_owned()))
        );
        assert_eq!(
            target(r#"{"targetUrn":"soundcloud:playlists:42"}"#),
            Some(Some("soundcloud:playlists:42".to_owned()))
        );
        assert_eq!(target(r#"{"targetUrn":"new:42"}"#), None);
        assert_eq!(target(r#"{"targetUrn":"42"}"#), None);
        assert_eq!(target(r#"{"targetUrn":"soundcloud:playlists:042"}"#), None);
        assert_eq!(
            target(&format!(
                r#"{{"targetUrn":"soundcloud:playlists:{}"}}"#,
                "1".repeat(130)
            )),
            None
        );
    }

    #[test]
    fn an_empty_body_retries_everything_and_a_bad_body_is_rejected() {
        assert_eq!(target(""), Some(None));
        assert_eq!(target(" \n"), Some(None));
        assert_eq!(target("{}"), Some(None));
        assert_eq!(target(r#"{"targetUrn":null}"#), Some(None));
        assert_eq!(target(r#"{"targetUrn":5}"#), None);
        assert_eq!(target("targetUrn=soundcloud:playlists:42"), None);
        assert_eq!(target("null"), None);
    }
}
