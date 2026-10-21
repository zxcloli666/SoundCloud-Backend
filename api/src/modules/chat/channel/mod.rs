mod answer;
mod auth_gate;
mod clock;
pub mod config_route;
mod dispatch;
mod exchange;
pub mod ock;
mod prepare;
mod replay;
pub mod sealed_auth;

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::post;
use tokio::sync::Semaphore;

use crate::modules::chat::ChatState;
use crate::modules::chat::channel::auth_gate::AuthGate;

pub const SEALED_AUTH_PATHS: [&str; 5] = [
    "/auth/login",
    "/auth/login/status",
    "/auth/link/create",
    "/auth/link/claim",
    "/auth/link/status",
];

pub const CONFIG_PATH: &str = "/chat/config";
pub const DEVICES_PATH: &str = "/chat/devices";
pub const WORK_PERMITS: usize = 2_048;

pub struct Channel {
    pub chat: Arc<ChatState>,
    pub inner: Router,
    busy: Semaphore,
    gate: AuthGate,
}

impl Channel {
    pub fn new(chat: Arc<ChatState>, inner: Router) -> Self {
        let permits = chat.config.max_parked.saturating_add(WORK_PERMITS);
        Self {
            chat,
            inner,
            busy: Semaphore::new(permits),
            gate: AuthGate::new(),
        }
    }
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

#[cfg(test)]
#[path = "../tests/channel_tests.rs"]
mod tests;
