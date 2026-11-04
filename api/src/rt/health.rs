use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use tracing::warn;

use crate::modules::chat::ChatState;
use crate::modules::health::handler::{self, HealthResponse};
use crate::rt::state::RtState;

const REDIS_TIMEOUT: Duration = Duration::from_secs(1);

pub fn router() -> Router<RtState> {
    Router::new()
        .route("/health", get(shallow))
        .route("/health/db", get(handler::check))
        .route("/health/live", get(handler::live))
        .route("/probe", get(tls_common::probe::probe))
}

async fn shallow(State(chat): State<Arc<ChatState>>) -> (StatusCode, Json<HealthResponse>) {
    handler::answer(redis_answers(&chat).await)
}

async fn redis_answers(chat: &ChatState) -> Result<(), &'static str> {
    let ping = async {
        let mut conn = chat.redis.get().await.ok()?;
        redis::cmd("PING")
            .query_async::<String>(&mut conn)
            .await
            .ok()
    };
    match tokio::time::timeout(REDIS_TIMEOUT, ping).await {
        Ok(Some(_)) => Ok(()),
        _ => {
            warn!("redis-rt does not answer the health ping");
            Err("redis_unavailable")
        }
    }
}
