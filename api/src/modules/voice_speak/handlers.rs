use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, FromRequest, Path, Request, State};
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use serde_json::json;
use uuid::Uuid;

use super::request::{BODY_LIMIT, SpeakBody, names_a_subject};
use super::service::{Busy, Fetched, Said};
use crate::common::admission::Endpoint;
use crate::common::session::SessionCtx;
use crate::common::voice_access::{admission_error, require_star};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub const REVISION_HEADER: &str = "x-voice-revision";
const UNAVAILABLE_RETRY_AFTER: i64 = 30;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/voice/speak",
            post(speak).layer(DefaultBodyLimit::max(BODY_LIMIT)),
        )
        .route("/voice/speak/{ticket}", get(fetch))
}

async fn speak(
    State(st): State<AppState>,
    ctx: SessionCtx,
    request: Request,
) -> AppResult<Response> {
    let live = st.voice_speak.live().ok_or_else(unavailable)?;
    let user = ctx.sc_user_id.as_str();
    if !names_a_subject(user) {
        return Err(invalid("the account id cannot name a line".to_owned()));
    }
    st.admission
        .check_key(Endpoint::VoiceSpeak, user)
        .await
        .map_err(|rejection| admission_error(rejection, "voice_speak_limited", "speak"))?;
    require_star(&st, user).await?;
    let Json(body) = Json::<SpeakBody>::from_request(request, &st)
        .await
        .map_err(unreadable)?;
    let line = body.into_line().map_err(invalid)?;
    live.say(user, line)
        .await
        .map(|said| answer(said, live.revision()))
        .map_err(|Busy| busy())
}

async fn fetch(
    State(st): State<AppState>,
    ctx: SessionCtx,
    Path(ticket): Path<String>,
) -> AppResult<Response> {
    let live = st.voice_speak.live().ok_or_else(unavailable)?;
    let ticket = ticket_of(&ticket)?;
    if !names_a_subject(&ctx.sc_user_id) {
        return Err(not_found());
    }
    reply(live.fetch(&ctx.sc_user_id, ticket).await, live.revision())
}

pub(super) fn answer(said: Said, revision: &str) -> Response {
    match said {
        Said::Cached(clip) => ogg(clip, revision),
        Said::Ticket(ticket) => (
            StatusCode::ACCEPTED,
            [(REVISION_HEADER, revision)],
            Json(json!({ "id": ticket.to_string() })),
        )
            .into_response(),
    }
}

pub(super) fn reply(fetched: Fetched, revision: &str) -> AppResult<Response> {
    match fetched {
        Fetched::Clip(clip) => Ok(ogg(clip, revision)),
        Fetched::Failed | Fetched::Expired => Err(AppError::coded(
            StatusCode::BAD_GATEWAY,
            "voice_speak_failed",
            "The line could not be voiced",
        )),
        Fetched::Pending => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

pub(super) fn ticket_of(raw: &str) -> AppResult<Uuid> {
    Uuid::parse_str(raw)
        .ok()
        .filter(|ticket| ticket.get_version_num() == 7)
        .ok_or_else(not_found)
}

fn not_found() -> AppError {
    AppError::coded(
        StatusCode::NOT_FOUND,
        "voice_speak_not_found",
        "No line has this ticket",
    )
}

fn ogg(clip: Bytes, revision: &str) -> Response {
    (
        [(CONTENT_TYPE, "audio/ogg")],
        [(REVISION_HEADER, revision)],
        clip,
    )
        .into_response()
}

fn unreadable(rejection: JsonRejection) -> AppError {
    let reason = match &rejection {
        JsonRejection::JsonSyntaxError(_) => "the body is not JSON",
        JsonRejection::JsonDataError(_) => "the body does not have the shape of a speak request",
        JsonRejection::MissingJsonContentType(_) => "the content type is not application/json",
        _ if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => "the body is too large",
        _ => "the body could not be read",
    };
    AppError::coded(rejection.status(), "voice_speak_invalid", reason)
}

fn invalid(reason: String) -> AppError {
    AppError::coded(StatusCode::BAD_REQUEST, "voice_speak_invalid", reason)
}

fn busy() -> AppError {
    AppError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        "voice_speak_busy",
        "Every voice is busy right now",
    )
}

fn unavailable() -> AppError {
    AppError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        "voice_speak_unavailable",
        "Server voicing is not available",
    )
    .with_retry_after(UNAVAILABLE_RETRY_AFTER)
}

#[cfg(test)]
#[path = "handlers_tests.rs"]
mod tests;
