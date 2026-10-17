use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use tokio::sync::Mutex;
use tracing::warn;

use crate::AppState;

const DATABASE_TIMEOUT: Duration = Duration::from_secs(2);
const CANARY_TIMEOUT: Duration = Duration::from_secs(5);
const READY_CACHE: Duration = Duration::from_secs(15);
const PROBE_SESSION: &str = "00000000-0000-0000-0000-000000000000";

pub type Readiness = Result<(), &'static str>;
pub type ReadyCache = Arc<Mutex<Option<(Instant, Readiness)>>>;

pub async fn ready(State(state): State<AppState>) -> (StatusCode, &'static str) {
    let mut last = state.readiness.lock().await;
    let readiness = match *last {
        Some((checked_at, readiness)) if checked_at.elapsed() < READY_CACHE => readiness,
        _ => {
            let (database, canary) = tokio::join!(sessions_answer(&state), canary_resolves(&state));
            let readiness = database.and(canary);
            if let Err(reason) = readiness {
                warn!("[ready] not ready: {reason}");
            }
            *last = Some((Instant::now(), readiness));
            readiness
        }
    };
    match readiness {
        Ok(()) => (StatusCode::OK, "ready"),
        Err(reason) => (StatusCode::SERVICE_UNAVAILABLE, reason),
    }
}

async fn sessions_answer(state: &AppState) -> Readiness {
    match tokio::time::timeout(DATABASE_TIMEOUT, state.pg.get_session(PROBE_SESSION)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => {
            warn!("[ready] session lookup failed: {e}");
            Err("database error")
        }
        Err(_) => Err("database timeout"),
    }
}

async fn canary_resolves(state: &AppState) -> Readiness {
    let Some(track_urn) = state.config.ready_canary_track.as_deref() else {
        return Ok(());
    };
    match tokio::time::timeout(CANARY_TIMEOUT, state.anon.fetch_track_meta(track_urn)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => {
            warn!("[ready] canary {track_urn} metadata failed: {e}");
            Err("canary metadata failed")
        }
        Err(_) => Err("canary metadata timeout"),
    }
}
