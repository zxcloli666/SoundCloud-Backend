use axum::extract::{Path, State};
use axum::routing::{get, patch};
use axum::{Json, Router};
use serde_json::{Value, json};

use crate::common::admin::AdminAuth;
use crate::error::AppResult;
use crate::modules::oauth_apps::dto::{CreateOAuthAppDto, OAuthAppResponse, UpdateOAuthAppDto};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/oauth-apps", get(find_all).post(create))
        .route("/oauth-apps/{id}", patch(update).delete(remove))
}

async fn find_all(
    _: AdminAuth,
    State(st): State<AppState>,
) -> AppResult<Json<Vec<OAuthAppResponse>>> {
    let apps = st.oauth_apps.find_all().await?;
    Ok(Json(apps.into_iter().map(OAuthAppResponse::from).collect()))
}

async fn create(
    _: AdminAuth,
    State(st): State<AppState>,
    Json(dto): Json<CreateOAuthAppDto>,
) -> AppResult<Json<OAuthAppResponse>> {
    let app = st.oauth_apps.create(&dto).await?;
    Ok(Json(app.into()))
}

async fn update(
    _: AdminAuth,
    State(st): State<AppState>,
    Path(id): Path<String>,
    Json(dto): Json<UpdateOAuthAppDto>,
) -> AppResult<Json<OAuthAppResponse>> {
    let app = st.oauth_apps.update(&id, &dto).await?;
    Ok(Json(app.into()))
}

async fn remove(
    _: AdminAuth,
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    st.oauth_apps.remove(&id).await?;
    Ok(Json(json!({ "success": true })))
}
