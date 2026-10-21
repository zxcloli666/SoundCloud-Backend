pub mod audit;
pub mod auth;
pub mod messages;
pub mod reports;
pub mod restore;
pub mod restrictions;
pub mod rewrap;
pub mod rooms;
pub mod stats;

use std::sync::Arc;

use axum::Router;
use axum::routing::{delete, get, post};

use crate::modules::chat::{ChatState, synthetic};

pub fn router<S>(chat: Arc<ChatState>) -> Router<S> {
    Router::new()
        .route("/chat-admin/reports", get(reports::list))
        .route(
            "/chat-admin/reports/{id}",
            get(reports::show).post(reports::resolve),
        )
        .route("/chat-admin/reports/{id}/audit", get(reports::trail))
        .route("/chat-admin/reports/{id}/restore", post(restore::restore))
        .route(
            "/chat-admin/restrictions",
            get(restrictions::list).post(restrictions::restrict),
        )
        .route(
            "/chat-admin/messages/{conversation}/{seq}",
            delete(messages::delete),
        )
        .route("/chat-admin/unlist/{code}", post(rooms::unlist))
        .route("/chat-admin/rewrap", post(rewrap::rewrap))
        .route("/chat-admin/stats", get(stats::stats))
        .route("/chat-admin/metrics", get(stats::metrics))
        .route("/chat-admin/loadtest/seed", post(synthetic::seed))
        .route("/chat-admin/loadtest/purge", post(synthetic::purge))
        .with_state(chat)
}

#[cfg(test)]
#[path = "../tests/admin_tests.rs"]
mod tests;
