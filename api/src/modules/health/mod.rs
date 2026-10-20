pub mod handler;

use axum::Router;
use axum::routing::get;

pub fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/health", get(handler::check))
        .route("/probe", get(tls_common::probe::probe))
}
