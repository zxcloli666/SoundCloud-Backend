pub mod audit;
pub mod auth;
pub mod cipher;
pub mod evidence;
pub mod filed;
pub mod media;
pub mod messages;
pub mod process;
pub mod reports;
pub mod resolve;
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
            get(reports::show).post(resolve::resolve),
        )
        .route("/chat-admin/reports/{id}/audit", get(reports::trail))
        .route(
            "/chat-admin/reports/{id}/media/{media_id}",
            get(media::view).head(media::head),
        )
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

#[cfg(test)]
#[path = "../tests/admin_media_support.rs"]
pub mod media_support;

#[cfg(test)]
#[path = "../tests/admin_media_fixtures.rs"]
pub mod media_fixtures;

#[cfg(test)]
#[path = "../tests/admin_media_tests.rs"]
mod media_tests;

#[cfg(test)]
#[path = "../tests/admin_backdrop_tests.rs"]
mod backdrop_tests;

#[cfg(test)]
#[path = "../tests/admin_restore_media_tests.rs"]
mod restore_media_tests;

#[cfg(test)]
#[path = "../tests/admin_s3_live_tests.rs"]
mod s3_live;

#[cfg(test)]
#[path = "../tests/admin_hold_tests.rs"]
mod hold_tests;

#[cfg(test)]
#[path = "../tests/admin_resolve_s3_live_tests.rs"]
mod resolve_s3_live;

#[cfg(test)]
#[path = "../tests/admin_audit_tests.rs"]
mod audit_tests;
