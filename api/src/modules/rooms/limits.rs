use axum::http::StatusCode;

use crate::common::admission::{AdmissionRejection, Endpoint};
use crate::common::session::SessionCtx;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

fn codes(endpoint: Endpoint) -> (&'static str, &'static str) {
    match endpoint {
        Endpoint::RoomCreate => ("rooms_create_limited", "rooms_create_busy"),
        Endpoint::RoomJoin => ("rooms_join_limited", "rooms_join_busy"),
        _ => ("rooms_list_limited", "rooms_list_busy"),
    }
}

fn refusal(endpoint: Endpoint, rejection: AdmissionRejection) -> AppError {
    let (limited, busy) = codes(endpoint);
    match rejection {
        AdmissionRejection::Limited {
            retry_after_seconds,
        } => AppError::coded(
            StatusCode::TOO_MANY_REQUESTS,
            limited,
            "Too many listening room requests",
        )
        .with_retry_after(i64::try_from(retry_after_seconds).unwrap_or(60)),
        AdmissionRejection::Unavailable => AppError::coded(
            StatusCode::SERVICE_UNAVAILABLE,
            busy,
            "Listening rooms are busy, try again shortly",
        )
        .with_retry_after(1),
    }
}

pub async fn admit(st: &AppState, ctx: &SessionCtx, endpoint: Endpoint) -> AppResult<()> {
    st.admission
        .check_session(endpoint, ctx.session_id)
        .await
        .map_err(|rejection| refusal(endpoint, rejection))
}
