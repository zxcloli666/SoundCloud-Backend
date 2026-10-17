use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use tracing::warn;

use crate::AppState;
use crate::backend::Backend;

const READY_TIMEOUT: Duration = Duration::from_secs(2);
const READY_CACHE: Duration = Duration::from_secs(15);
const PROBE_KEY: &str = "ready-probe";

pub type Readiness = Result<(), &'static str>;

pub async fn health() -> StatusCode {
    StatusCode::OK
}

pub async fn ready(State(state): State<Arc<AppState>>) -> (StatusCode, &'static str) {
    let mut last = state.readiness.lock().await;
    let readiness = match *last {
        Some((checked_at, readiness)) if checked_at.elapsed() < READY_CACHE => readiness,
        _ => {
            let readiness = check(&state.backend, state.config.ready_canary_key.as_deref()).await;
            *last = Some((Instant::now(), readiness));
            readiness
        }
    };
    match readiness {
        Ok(()) => (StatusCode::OK, "ready"),
        Err(reason) => (StatusCode::SERVICE_UNAVAILABLE, reason),
    }
}

async fn check(backend: &Backend, canary: Option<&str>) -> Readiness {
    let key = canary.unwrap_or(PROBE_KEY);
    let readiness = match tokio::time::timeout(READY_TIMEOUT, backend.head(key)).await {
        Err(_) => Err("backend timeout"),
        Ok(Err(e)) => {
            warn!("[ready] head {key} failed: {e}");
            Err("backend error")
        }
        Ok(Ok(None)) if canary.is_some() => Err("canary missing"),
        Ok(Ok(_)) => Ok(()),
    };
    if let Err(reason) = readiness {
        warn!("[ready] not ready: {reason}");
    }
    readiness
}

#[cfg(test)]
mod tests {
    use super::check;
    use crate::backend::{Backend, LocalBackend};

    async fn local_backend() -> (Backend, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("storage-ready-{}", uuid::Uuid::new_v4()));
        let backend = LocalBackend::new(root.to_str().unwrap()).await.unwrap();
        (Backend::Local(Box::new(backend)), root)
    }

    #[tokio::test]
    async fn without_a_canary_an_answering_backend_is_ready() {
        let (backend, root) = local_backend().await;
        assert_eq!(check(&backend, None).await, Ok(()));
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn a_missing_canary_is_not_ready() {
        let (backend, root) = local_backend().await;
        assert_eq!(
            check(&backend, Some("soundcloud_tracks_42.m4a")).await,
            Err("canary missing")
        );
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn a_present_canary_is_ready() {
        let (backend, root) = local_backend().await;
        tokio::fs::write(root.join("soundcloud_tracks_42.m4a"), b"audio")
            .await
            .unwrap();
        assert_eq!(
            check(&backend, Some("soundcloud_tracks_42.m4a")).await,
            Ok(())
        );
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn a_backend_that_fails_the_read_is_not_ready() {
        let (backend, root) = local_backend().await;
        assert_eq!(
            check(&backend, Some("bad\0key")).await,
            Err("backend error")
        );
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
