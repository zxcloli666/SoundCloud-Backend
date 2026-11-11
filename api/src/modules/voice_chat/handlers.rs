use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, FromRequest, Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};

use super::VoiceChat;
use super::request::{ChatBody, body_limit};
use crate::common::admission::Endpoint;
use crate::common::session::SessionCtx;
use crate::error::{AppError, AppResult};
use crate::modules::remote::handlers::{admission_error, require_star};
use crate::router::MAX_BODY_BYTES;
use crate::state::AppState;

const UNAVAILABLE_RETRY_AFTER: i64 = 30;

pub fn router(voice_chat: &VoiceChat) -> Router<AppState> {
    let limit = voice_chat
        .live()
        .map_or(MAX_BODY_BYTES, |live| body_limit(live.limits()))
        .min(MAX_BODY_BYTES);
    Router::new().route(
        "/voice/chat",
        post(chat).layer(DefaultBodyLimit::max(limit)),
    )
}

async fn chat(
    State(st): State<AppState>,
    ctx: SessionCtx,
    request: Request,
) -> AppResult<Response> {
    let chat = st.voice_chat.live().ok_or_else(unavailable)?;
    let user = ctx.sc_user_id.as_str();
    st.admission
        .check_key(Endpoint::VoiceChat, user)
        .await
        .map_err(|rejection| admission_error(rejection, "voice_chat_limited", "chat"))?;
    require_star(&st, user).await?;
    let Json(body) = Json::<ChatBody>::from_request(request, &st)
        .await
        .map_err(unreadable)?;
    let turn = body.into_turn(chat.limits()).map_err(invalid)?;
    let request = chat.request(&turn, user).map_err(invalid)?;
    chat.answer(request, turn.tools)
        .await
        .map_err(|_| unavailable())
}

fn unreadable(rejection: JsonRejection) -> AppError {
    let reason = match &rejection {
        JsonRejection::JsonSyntaxError(_) => "the body is not JSON",
        JsonRejection::JsonDataError(_) => "the body does not have the shape of a chat request",
        JsonRejection::MissingJsonContentType(_) => "the content type is not application/json",
        _ if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE => "the body is too large",
        _ => "the body could not be read",
    };
    AppError::coded(rejection.status(), "voice_chat_invalid", reason)
}

fn invalid(reason: String) -> AppError {
    AppError::coded(StatusCode::BAD_REQUEST, "voice_chat_invalid", reason)
}

fn unavailable() -> AppError {
    AppError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        "chat_unavailable",
        "The chat is not available right now",
    )
    .with_retry_after(UNAVAILABLE_RETRY_AFTER)
}
