mod alice;
mod converse;
mod phrases;
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
}
