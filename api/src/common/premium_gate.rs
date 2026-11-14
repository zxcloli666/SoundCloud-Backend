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
        || path == "/health/live"
        || path == "/probe"
        || path == "/me/subscription"
        || path == "/admin/metrics"
        || path.starts_with("/auth/")
        || path.starts_with("/oauth/")
        || path.starts_with("/skills/")
        || path.starts_with("/remote/")
        || path == "/voice/chat"
        || path == "/voice/speak"
        || path.starts_with("/voice/speak/")
        || path.starts_with("/.well-known/")
        || path.starts_with("/diag/")
        || path.starts_with("/admin/diag/")
}

#[cfg(test)]
mod tests {
    use super::is_open_path;

    #[test]
    fn voice_paths_check_star_in_their_own_handlers() {
        for path in [
            "/remote/wait",
            "/remote/links",
            "/remote/links/1",
            "/voice/chat",
            "/voice/speak",
            "/voice/speak/0190f0c2-7a4e-7c1d-9b3e-5f2a8c6d4e1b",
            "/skills/salut/token",
            "/oauth/authorize/status",
        ] {
            assert!(is_open_path(path), "{path}");
        }
        assert!(!is_open_path("/search/resolve"));
        assert!(!is_open_path("/voice/other"));
        assert!(!is_open_path("/voice/chat/extra"));
        assert!(!is_open_path("/voice/speaker"));
        assert!(!is_open_path("/me/likes"));
    }
}
