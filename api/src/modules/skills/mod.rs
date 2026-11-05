mod alice;
mod converse;
mod phrases;
mod review;
mod salut;
mod speech;

#[cfg(test)]
mod skills_tests;

use axum::Router;
use axum::routing::post;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/skills/alice", post(alice::webhook))
        .route("/skills/salut", post(salut::webhook))
        .route("/skills/salut/{token}", post(salut::webhook_with_token))
}
