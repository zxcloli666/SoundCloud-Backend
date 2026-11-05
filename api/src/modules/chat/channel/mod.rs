pub mod answer;
pub mod clock;
pub mod config_route;
pub mod dispatch;
mod exchange;
pub mod ock;
mod prepare;
mod replay;

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::post;
use tokio::sync::Semaphore;

use crate::modules::chat::ChatState;

pub const CONFIG_PATH: &str = "/chat/config";
pub const DEVICES_PATH: &str = "/chat/devices";
pub const WORK_PERMITS: usize = 2_048;

pub struct Channel {
    pub chat: Arc<ChatState>,
    pub inner: Router,
    busy: Semaphore,
}

impl Channel {
    pub fn new(chat: Arc<ChatState>, inner: Router) -> Self {
        let permits = chat.config.max_parked.saturating_add(WORK_PERMITS);
        Self {
            chat,
            inner,
            busy: Semaphore::new(permits),
        }
    }
}

pub fn router<S>(channel: Arc<Channel>) -> Router<S> {
    Router::new()
        .route(
            "/chat/x",
            post(exchange::exchange).layer(DefaultBodyLimit::max(sc_chat_wire::seal::MAX_ENVELOPE)),
        )
        .with_state(channel)
}

#[cfg(test)]
#[path = "../tests/channel_tests.rs"]
mod tests;
