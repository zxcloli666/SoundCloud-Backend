use axum::extract::{FromRequestParts, Request, State};
use axum::http::Method;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::common::admin::AdminAuth;
use crate::common::session::SessionCtx;
use crate::error::AppError;
use crate::state::AppState;

pub async fn premium_gate(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if !state.config.premium_reserve {
        return next.run(req).await;
    }
    if req.method() == Method::OPTIONS || is_open_path(req.uri().path()) {
        return next.run(req).await;
    }

    let (mut parts, body) = req.into_parts();
    if AdminAuth::from_request_parts(&mut parts, &state)
        .await
        .is_ok()
    {
        return next.run(Request::from_parts(parts, body)).await;
    }
    let ctx = match SessionCtx::from_request_parts(&mut parts, &state).await {
        Ok(c) => c,
        Err(e) => return e.into_response(),
    };
    match state.subscriptions.is_premium(&ctx.sc_user_id).await {
        Ok(true) => {}
        Ok(false) => return AppError::forbidden("Star subscription required").into_response(),
        Err(e) => return e.into_response(),
    }

    next.run(Request::from_parts(parts, body)).await
}

fn is_open_path(path: &str) -> bool {
    path == "/health"
        || path == "/me/subscription"
        || path.starts_with("/auth/")
        || path.starts_with("/.well-known/")
}
