use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use backend_contracts::remote::{
    LinkCodeRequest, LinkCodeResponse, LinksResponse, MAX_REPLY_BYTES, RemoteReply, WAIT_HOLD_SECS,
    WaitResponse,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::hub::{Refused, Replied};
use crate::common::admission::{AdmissionRejection, Endpoint};
use crate::common::session::SessionCtx;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

const MAX_DEVICE_LEN: usize = 64;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/remote/wait", get(wait))
        .route(
            "/remote/reply",
            post(reply).layer(DefaultBodyLimit::max(MAX_REPLY_BYTES)),
        )
        .route("/remote/link-code", post(link_code))
        .route("/remote/links", get(links))
        .route("/remote/links/{id}", delete(revoke))
}

#[derive(Debug, Deserialize)]
struct WaitQuery {
    #[serde(default)]
    since: Option<String>,
    #[serde(default)]
    device: Option<String>,
    #[serde(default)]
    active: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReplyBody {
    id: Uuid,
    ok: bool,
    #[serde(default)]
    speech: String,
    #[serde(default)]
    display: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    state: Option<Value>,
}

fn number(raw: Option<String>, name: &str) -> AppResult<Option<i64>> {
    match raw.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(text) => text
            .parse()
            .map(Some)
            .map_err(|_| AppError::bad_request(format!("{name} is not a number"))),
    }
}

pub fn admission_error(rejection: AdmissionRejection, code: &'static str, what: &str) -> AppError {
    match rejection {
        AdmissionRejection::Limited {
            retry_after_seconds,
        } => AppError::coded(
            StatusCode::TOO_MANY_REQUESTS,
            code,
            format!("Too many {what} requests"),
        )
        .with_retry_after(i64::try_from(retry_after_seconds).unwrap_or(60)),
        AdmissionRejection::Unavailable => AppError::coded(
            StatusCode::SERVICE_UNAVAILABLE,
            "voice_busy",
            format!("{what} is busy, try again shortly"),
        )
        .with_retry_after(1),
    }
}

pub fn voice_disabled() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "statusCode": 503,
            "code": "voice_disabled",
            "message": "Voice is not configured",
            "error": "Service Unavailable",
        })),
    )
        .into_response()
}

pub fn star_required() -> AppError {
    AppError::coded(
        StatusCode::FORBIDDEN,
        "star_required",
        "Star subscription required",
    )
}

pub async fn require_star(st: &AppState, user: &str) -> AppResult<()> {
    if st.subscriptions.is_premium_cached(user).await? {
        Ok(())
    } else {
        Err(star_required())
    }
}

async fn wait(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Query(q): Query<WaitQuery>,
) -> AppResult<Json<WaitResponse>> {
    let since = number(q.since, "since")?;
    let active = number(q.active, "active")?.unwrap_or(0);
    st.admission
        .check_session(Endpoint::VoiceRemoteWait, ctx.session_id)
        .await
        .map_err(|rejection| admission_error(rejection, "voice_remote_limited", "remote wait"))?;
    require_star(&st, &ctx.sc_user_id).await?;
    let device: String = q
        .device
        .unwrap_or_default()
        .chars()
        .take(MAX_DEVICE_LEN)
        .collect();
    let hold = Duration::from_secs(WAIT_HOLD_SECS);
    match st
        .remote
        .wait(&ctx.sc_user_id, &device, active, since, hold)
        .await
    {
        Ok(response) => Ok(Json(response)),
        Err(Refused::TooManyDevices) => Err(AppError::coded(
            StatusCode::TOO_MANY_REQUESTS,
            "voice_too_many_devices",
            "Too many devices are waiting for this account",
        )
        .with_retry_after(20)),
        Err(Refused::Unavailable) => Err(AppError::coded(
            StatusCode::SERVICE_UNAVAILABLE,
            "voice_busy",
            "remote is busy, try again shortly",
        )
        .with_retry_after(5)),
    }
}

async fn reply(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Json(body): Json<ReplyBody>,
) -> AppResult<StatusCode> {
    st.admission
        .check_session(Endpoint::VoiceRemote, ctx.session_id)
        .await
        .map_err(|rejection| admission_error(rejection, "voice_remote_limited", "remote"))?;
    require_star(&st, &ctx.sc_user_id).await?;
    let display = if body.display.is_empty() {
        body.text
    } else {
        body.display
    };
    let replied = st
        .remote
        .reply(
            &ctx.sc_user_id,
            RemoteReply {
                id: body.id,
                ok: body.ok,
                speech: body.speech,
                display,
                state: body.state,
            },
        )
        .await;
    match replied {
        Replied::Recorded => Ok(StatusCode::NO_CONTENT),
        Replied::Unknown => Err(AppError::coded(
            StatusCode::BAD_REQUEST,
            "voice_reply_unknown",
            "this command is not waiting for an answer",
        )),
        Replied::Busy => Err(AppError::coded(
            StatusCode::SERVICE_UNAVAILABLE,
            "voice_busy",
            "remote is busy, try again shortly",
        )
        .with_retry_after(1)),
    }
}

async fn link_code(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Json(body): Json<LinkCodeRequest>,
) -> AppResult<Json<LinkCodeResponse>> {
    st.admission
        .check_session(Endpoint::VoiceLinkCode, ctx.session_id)
        .await
        .map_err(|rejection| admission_error(rejection, "voice_link_limited", "link code"))?;
    require_star(&st, &ctx.sc_user_id).await?;
    Ok(Json(
        st.links.create_code(&ctx.sc_user_id, body.provider).await?,
    ))
}

async fn links(State(st): State<AppState>, ctx: SessionCtx) -> AppResult<Json<LinksResponse>> {
    Ok(Json(LinksResponse {
        links: st.links.list(&ctx.sc_user_id).await?,
    }))
}

async fn revoke(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(id): Path<Uuid>,
) -> AppResult<StatusCode> {
    if st.links.revoke(&ctx.sc_user_id, id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::not_found("Link not found"))
    }
}
