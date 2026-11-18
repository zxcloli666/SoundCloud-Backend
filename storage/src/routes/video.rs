use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use futures::StreamExt;
use subtle::ConstantTimeEq;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};
use uuid::Uuid;

use crate::AppState;

const MAX_VIDEO_BYTES: u64 = 96 * 1024 * 1024;

pub async fn put(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    body: Body,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if state.config.disable_upload {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "upload disabled on this host".into(),
        ));
    }
    authorize(&state, &headers)?;
    if !valid_name(&name) {
        return Err((StatusCode::BAD_REQUEST, "invalid video name".into()));
    }
    let tmp = std::path::PathBuf::from(state.config.source_path())
        .join(format!("{}.video", Uuid::new_v4()));
    let outcome = receive(body, &tmp).await;
    let size = match outcome {
        Ok(size) => size,
        Err(error) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(error);
        }
    };
    let key = format!("video/{name}");
    if let Err(error) = state.backend.put_file(&key, &tmp).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        warn!("[video] put {key} failed: {error}");
        return Err((StatusCode::BAD_GATEWAY, "backend write failed".into()));
    }
    info!("[video] stored {key} bytes={size}");
    Ok((StatusCode::CREATED, key))
}

fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), (StatusCode, String)> {
    if state.config.video_upload_token.is_empty() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "video upload token is not configured".into(),
        ));
    }
    check_bearer(headers, &state.config.video_upload_token)
}

fn check_bearer(headers: &HeaderMap, expected: &str) -> Result<(), (StatusCode, String)> {
    let token = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or((StatusCode::UNAUTHORIZED, "missing token".into()))?;
    if token.as_bytes().ct_eq(expected.as_bytes()).unwrap_u8() != 1 {
        return Err((StatusCode::FORBIDDEN, "invalid token".into()));
    }
    Ok(())
}

pub(crate) fn valid_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".mp4") else {
        return false;
    };
    let Some((track, attempt)) = stem.split_once('-') else {
        return false;
    };
    let digits = |part: &str| {
        !part.is_empty() && part.len() <= 19 && part.bytes().all(|byte| byte.is_ascii_digit())
    };
    digits(track) && digits(attempt) && !track.starts_with('0')
}

async fn receive(body: Body, tmp: &std::path::Path) -> Result<u64, (StatusCode, String)> {
    let mut file = tokio::fs::File::create(tmp)
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let mut stream = body.into_data_stream();
    let mut written = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| (StatusCode::BAD_REQUEST, error.to_string()))?;
        written += chunk.len() as u64;
        if written > MAX_VIDEO_BYTES {
            return Err((StatusCode::PAYLOAD_TOO_LARGE, "video is too large".into()));
        }
        file.write_all(&chunk)
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    }
    file.flush()
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    if written == 0 {
        return Err((StatusCode::BAD_REQUEST, "empty body".into()));
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, StatusCode};

    use super::{check_bearer, valid_name};

    fn bearer(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        headers
    }

    #[test]
    fn the_video_token_opens_a_video_put() {
        assert!(check_bearer(&bearer("video-secret"), "video-secret").is_ok());
    }

    #[test]
    fn a_wrong_token_is_refused() {
        let error = check_bearer(&bearer("other"), "video-secret").unwrap_err();
        assert_eq!(error.0, StatusCode::FORBIDDEN);
    }

    #[test]
    fn the_admin_token_is_not_the_video_token() {
        let error = check_bearer(&bearer("admin-secret"), "video-secret").unwrap_err();
        assert_eq!(error.0, StatusCode::FORBIDDEN);
    }

    #[test]
    fn a_missing_header_is_unauthorized() {
        let error = check_bearer(&HeaderMap::new(), "video-secret").unwrap_err();
        assert_eq!(error.0, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn a_name_is_track_dash_attempt_dot_mp4() {
        assert!(valid_name("1304903974-2.mp4"));
        assert!(!valid_name("../1304903974-2.mp4"));
        assert!(!valid_name("1304903974.mp4"));
        assert!(!valid_name("0123-1.mp4"));
        assert!(!valid_name("12-1.m4a"));
        assert!(!valid_name("12-1-3.mp4"));
    }
}
