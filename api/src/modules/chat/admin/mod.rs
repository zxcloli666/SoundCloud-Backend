pub mod auth;
pub mod handlers;

use std::sync::Arc;

use axum::Router;
use axum::routing::{delete, get, post};

use crate::modules::chat::{ChatState, synthetic};

pub fn router<S>(chat: Arc<ChatState>) -> Router<S> {
    Router::new()
        .route("/chat-admin/reports", get(handlers::reports))
        .route(
            "/chat-admin/reports/{id}",
            get(handlers::report).post(handlers::resolve_report),
        )
        .route(
            "/chat-admin/restrictions",
            get(handlers::restrictions).post(handlers::restrict),
        )
        .route(
            "/chat-admin/messages/{conversation}/{seq}",
            delete(handlers::delete_message),
        )
        .route("/chat-admin/unlist/{code}", post(handlers::unlist))
        .route("/chat-admin/rewrap", post(handlers::rewrap))
        .route("/chat-admin/stats", get(handlers::stats))
        .route("/chat-admin/loadtest/seed", post(synthetic::seed))
        .route("/chat-admin/loadtest/purge", post(synthetic::purge))
        .with_state(chat)
}
