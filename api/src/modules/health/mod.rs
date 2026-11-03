pub mod handler;

use axum::Router;
use axum::extract::FromRef;
use axum::routing::get;
use sqlx::PgPool;

pub fn router<S>() -> Router<S>
where
    PgPool: FromRef<S>,
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/health", get(handler::check))
        .route("/health/live", get(handler::live))
        .route("/probe", get(tls_common::probe::probe))
}
