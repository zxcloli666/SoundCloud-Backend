use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::from_fn_with_state;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use tracing::error;

use crate::common::admission::{PublicAdmission, admit_link_create, admit_login};
use crate::common::session::RawSessionIdHeader;
use crate::common::uuid::parse_uuid;
use crate::error::{AppError, AppResult};
use crate::modules::auth::callback_page::{CallbackPageParams, render as render_callback_page};
use crate::modules::auth::dto::*;
use crate::modules::auth::service::{RefreshAttempt, RefreshOutcome};
use crate::state::AppState;

pub fn router(admission: Arc<PublicAdmission>) -> Router<AppState> {
    Router::new()
        .route(
            "/auth/login",
            get(login).route_layer(from_fn_with_state(admission.clone(), admit_login)),
        )
        .route("/auth/login/status", get(login_status))
        .route("/auth/callback", get(callback))
        .route("/auth/session", get(session))
        .route("/auth/soundcloud", get(soundcloud_status))
        .route("/auth/soundcloud/refresh", post(soundcloud_refresh))
        .route("/auth/logout", post(logout))
        .route(
            "/auth/link/create",
            post(link_create).route_layer(from_fn_with_state(admission, admit_link_create)),
        )
        .route("/auth/link/claim", post(link_claim))
        .route("/auth/link/status", get(link_status))
}

#[tracing::instrument(skip_all)]
async fn login(
    State(state): State<AppState>,
    RawSessionIdHeader(raw): RawSessionIdHeader,
) -> AppResult<Response> {
    let result = state
        .auth
        .initiate_login(raw.as_deref().and_then(parse_uuid))
        .await?;
    Ok(no_store_json(LoginResponse {
        url: result.url,
        login_request_id: result.login_request_id,
    }))
}

#[tracing::instrument(skip_all)]
async fn login_status(
    State(state): State<AppState>,
    Query(query): Query<LoginStatusQuery>,
) -> AppResult<Response> {
    let Some(id) = parse_uuid(&query.id) else {
        return Ok(no_store_json(json!({
            "status": "expired",
            "error": "Invalid login request id",
        })));
    };
    let status = state.auth.get_login_request_status(id).await?;
    serde_json::to_value(status)
        .map(no_store_json)
        .map_err(|error| AppError::internal(format!("encode login status: {error}")))
}

#[tracing::instrument(skip_all)]
async fn callback(State(state): State<AppState>, Query(query): Query<CallbackQuery>) -> Response {
    let html = match state
        .auth
        .handle_callback(&state.resolve, &query.code, &query.state)
        .await
    {
        Ok(result) => {
            let login_request_id = result.login_request_id.map(|id| id.to_string());
            render_callback_page(&CallbackPageParams {
                login_request_id: login_request_id.as_deref(),
                initial_status: &result.initial_status,
                username: result.username.as_deref(),
                error: result.error.as_deref(),
            })
        }
        Err(error) => {
            error!(%error, "Unhandled OAuth callback error");
            render_callback_page(&CallbackPageParams {
                login_request_id: None,
                initial_status: "failed",
                username: None,
                error: Some("Authentication failed. Please try again."),
            })
        }
    };

    let mut response = (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
        .into_response();
    insert_no_store(&mut response);
    response
}

#[tracing::instrument(skip_all)]
async fn session(
    State(state): State<AppState>,
    RawSessionIdHeader(raw): RawSessionIdHeader,
) -> AppResult<Response> {
    let Some(session_id) = raw.as_deref().and_then(parse_uuid) else {
        return Ok(no_store_json(unauthenticated_session()));
    };
    let Some(session) = state.auth.get_auth_session(session_id).await? else {
        return Ok(no_store_json(unauthenticated_session()));
    };
    let Some(soundcloud_user_id) = session.soundcloud_user_id else {
        return Ok(no_store_json(unauthenticated_session()));
    };
    let soundcloud_user_urn = crate::common::sc_ids::user_urn(&soundcloud_user_id);

    Ok(no_store_json(SessionResponse {
        authenticated: true,
        session_id: Some(session.id),
        username: session.username,
        soundcloud_user_id: Some(soundcloud_user_urn),
        expires_at: session.expires_at,
    }))
}

#[tracing::instrument(skip_all)]
async fn soundcloud_status(
    State(state): State<AppState>,
    RawSessionIdHeader(raw): RawSessionIdHeader,
) -> AppResult<Response> {
    let session_id = required_session_id(raw.as_deref())?;
    Ok(no_store_json(
        state.auth.soundcloud_status(session_id).await?,
    ))
}

#[tracing::instrument(skip_all)]
async fn soundcloud_refresh(
    State(state): State<AppState>,
    RawSessionIdHeader(raw): RawSessionIdHeader,
) -> AppResult<Response> {
    let session_id = required_session_id(raw.as_deref())?;
    let attempt = state.auth.refresh_soundcloud(session_id).await?;
    Ok(refresh_response(attempt))
}

#[tracing::instrument(skip_all)]
async fn logout(
    State(state): State<AppState>,
    RawSessionIdHeader(raw): RawSessionIdHeader,
) -> AppResult<Response> {
    if let Some(session_id) = raw.as_deref().and_then(parse_uuid) {
        state.auth.logout(session_id).await?;
    }
    Ok(no_store_json(LogoutResponse { success: true }))
}

#[tracing::instrument(skip_all)]
async fn link_create(
    State(state): State<AppState>,
    RawSessionIdHeader(raw): RawSessionIdHeader,
    Json(body): Json<CreateLinkRequest>,
) -> AppResult<Response> {
    let result = state
        .link
        .create(&body.mode, raw.as_deref().and_then(parse_uuid))
        .await?;
    Ok(no_store_json(CreateLinkResponse {
        link_request_id: result.link_request_id,
        claim_token: result.claim_token,
        expires_at: result.expires_at,
    }))
}

#[tracing::instrument(skip_all)]
async fn link_claim(
    State(state): State<AppState>,
    RawSessionIdHeader(raw): RawSessionIdHeader,
    Json(body): Json<ClaimLinkRequest>,
) -> AppResult<Response> {
    let result = state
        .link
        .claim(&body.claim_token, raw.as_deref().and_then(parse_uuid))
        .await?;
    Ok(no_store_json(ClaimLinkResponse {
        session_id: result.session_id,
        mode: result.mode,
    }))
}

#[tracing::instrument(skip_all)]
async fn link_status(
    State(state): State<AppState>,
    Query(query): Query<LinkStatusQuery>,
) -> AppResult<Response> {
    let Some(id) = parse_uuid(&query.id) else {
        return Ok(no_store_json(LinkStatusResponse {
            status: "expired".to_owned(),
            mode: "pull".to_owned(),
            session_id: None,
            error: Some("Unknown link request".to_owned()),
        }));
    };
    let result = state.link.get_status(id).await?;
    Ok(no_store_json(LinkStatusResponse {
        status: result.status,
        mode: result.mode,
        session_id: result.session_id,
        error: result.error,
    }))
}

fn refresh_response(attempt: RefreshAttempt) -> Response {
    let body = attempt.response();
    let retry_after = body.retry_after_sec;
    let status = match attempt.outcome {
        RefreshOutcome::Refreshed | RefreshOutcome::AlreadyFresh => StatusCode::OK,
        RefreshOutcome::InProgress => StatusCode::ACCEPTED,
        RefreshOutcome::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        RefreshOutcome::ReauthorizationRequired | RefreshOutcome::NotConnected => {
            StatusCode::CONFLICT
        }
        RefreshOutcome::TimedOut => StatusCode::GATEWAY_TIMEOUT,
        RefreshOutcome::RetryLater => StatusCode::BAD_GATEWAY,
    };
    let mut response = (status, Json(body)).into_response();
    insert_retry_after(&mut response, retry_after);
    insert_no_store(&mut response);
    response
}

fn insert_retry_after(response: &mut Response, retry_after: Option<i64>) {
    if let Some(seconds) = retry_after
        && let Ok(value) = HeaderValue::from_str(&seconds.max(1).to_string())
    {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
}

fn no_store_json<T: serde::Serialize>(value: T) -> Response {
    let mut response = Json(value).into_response();
    insert_no_store(&mut response);
    response
}

fn insert_no_store(response: &mut Response) {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private"),
    );
}

fn required_session_id(raw: Option<&str>) -> AppResult<uuid::Uuid> {
    raw.and_then(parse_uuid)
        .ok_or_else(|| AppError::unauthorized("Missing or malformed session id"))
}

fn unauthenticated_session() -> SessionResponse {
    SessionResponse {
        authenticated: false,
        session_id: None,
        username: None,
        soundcloud_user_id: None,
        expires_at: None,
    }
}

#[cfg(test)]
mod tests;
