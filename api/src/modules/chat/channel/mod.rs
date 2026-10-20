mod answer;
pub mod config_route;
mod dispatch;
mod exchange;
pub mod ock;

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::post;

use crate::modules::chat::ChatState;

pub const SEALED_AUTH_PATHS: [&str; 5] = [
    "/auth/login",
    "/auth/login/status",
    "/auth/link/create",
    "/auth/link/claim",
    "/auth/link/status",
];

pub struct Channel {
    pub chat: Arc<ChatState>,
    pub inner: Router,
}

pub fn router<S>(channel: Arc<Channel>) -> Router<S> {
    Router::new()
        .route(
            "/chat/x",
            post(exchange::exchange).layer(DefaultBodyLimit::max(chat_wire::seal::MAX_ENVELOPE)),
        )
        .with_state(channel)
}

pub fn is_sealed_auth(path: &str) -> bool {
    SEALED_AUTH_PATHS.contains(&path)
}
