use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};

use super::request::ChatBody;
use crate::common::admission::Endpoint;
use crate::common::session::SessionCtx;
use crate::error::{AppError, AppResult};
use crate::modules::remote::handlers::{admission_error, require_star};
use crate::state::AppState;

const UNAVAILABLE_RETRY_AFTER: i64 = 30;

pub fn router() -> Router<AppState> {
    Router::new().route("/voice/chat", post(chat))
}

async fn chat(
    State(st): State<AppState>,
    ctx: SessionCtx,
    body: Result<Json<ChatBody>, JsonRejection>,
) -> AppResult<Response> {
    let chat = st.voice_chat.live().ok_or_else(unavailable)?;
    st.admission
        .check_key(Endpoint::VoiceChat, &ctx.sc_user_id)
        .await
        .map_err(|rejection| admission_error(rejection, "voice_chat_limited", "chat"))?;
    require_star(&st, &ctx.sc_user_id).await?;
    let Json(body) = body.map_err(unreadable)?;
    let turn = body.into_turn(chat.limits()).map_err(invalid)?;
    chat.answer(turn, &ctx.sc_user_id)
        .await
        .map_err(|_| unavailable())
}

fn unreadable(rejection: JsonRejection) -> AppError {
    let reason = match &rejection {
        JsonRejection::JsonSyntaxError(_) => "the body is not JSON",
        JsonRejection::JsonDataError(_) => "the body does not have the shape of a chat request",
        JsonRejection::MissingJsonContentType(_) => "the content type is not application/json",
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
