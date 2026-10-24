use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use futures::StreamExt;
use subtle::ConstantTimeEq;
use tracing::{info, warn};

use crate::AppState;
use crate::backend::{Backend, BackendError, ByteStream, ObjectInfo};

const REDIRECT_PRESIGN_EXPIRES: Duration = Duration::from_secs(15 * 60);

fn validate_path(path: &str) -> Result<String, StatusCode> {
    if path.contains("..") || path.starts_with('/') {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(object_path(path))
}

fn object_path(path: &str) -> String {
    path.strip_suffix(".m4a")
        .and_then(crate::backend::canonical_track_filename)
        .map_or_else(|| path.to_owned(), |stem| crate::backend::key_for(&stem))
}

/// GET /{path} — stream bytes to client (never redirect, even for S3 backend).
pub async fn serve(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Result<Response, StatusCode> {
    let path = validate_path(&path)?;

    let (info, stream) = match with_first_bytes(|| state.backend.stream(&path)).await {
        Ok(v) => v,
        Err(BackendError::NotFound) => return Err(StatusCode::NOT_FOUND),
        Err(e) => {
            warn!("[files] stream {path} failed: {e}");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            info.content_type
                .as_deref()
                .unwrap_or("application/octet-stream"),
        )
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .header(header::ACCEPT_RANGES, "bytes");
    if info.size > 0 {
        builder = builder.header(header::CONTENT_LENGTH, info.size);
    }

    Ok(builder.body(Body::from_stream(stream)).unwrap())
}

const OPEN_ATTEMPTS: usize = 2;

async fn with_first_bytes<F, Fut>(open: F) -> Result<(ObjectInfo, ByteStream), BackendError>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<(ObjectInfo, ByteStream), BackendError>>,
{
    let mut last = None;
    for _ in 0..OPEN_ATTEMPTS {
        let (info, mut stream) = open().await?;
        match stream.next().await {
            Some(Ok(first)) => {
                let rest = futures::stream::once(async move { Ok(first) }).chain(stream);
                return Ok((info, Box::pin(rest)));
            }
            Some(Err(e)) => last = Some(e),
            None => return Ok((info, stream)),
        }
    }
    Err(BackendError::Io(last.unwrap_or_else(|| {
        std::io::Error::other("no bytes before the first chunk")
    })))
}

/// GET /redirect/{path} — stable URL for AI pipeline workers.
/// S3 backend: 307 → freshly-signed presigned URL (worker follows to S3 directly).
/// Local backend: stream bytes from storage itself.
pub async fn redirect(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Result<Response, StatusCode> {
    let path = validate_path(&path)?;

    match &*state.backend {
        Backend::S3(s3) => match s3.presign_get(&path, REDIRECT_PRESIGN_EXPIRES).await {
            Ok(url) => Ok(Redirect::temporary(&url).into_response()),
            Err(e) => {
                warn!("[files] redirect presign {path} failed: {e}");
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        },
        Backend::Gdrive(gd) => match gd.public_link(&path).await {
            Ok(url) => Ok(Redirect::temporary(&url).into_response()),
            Err(BackendError::NotFound) => Err(StatusCode::NOT_FOUND),
            Err(e) => {
                warn!("[files] redirect gdrive link {path} failed: {e}");
                Err(StatusCode::INTERNAL_SERVER_ERROR)
            }
        },
        Backend::Local(_) => serve(State(state), Path(path)).await,
    }
}

/// HEAD /{path} — existence + size check only (no body download from S3).
pub async fn head(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
) -> Result<Response, StatusCode> {
    let path = validate_path(&path)?;

    let info = match state.backend.head(&path).await {
        Ok(Some(info)) => info,
        Ok(None) => return Err(StatusCode::NOT_FOUND),
        Err(e) => {
            warn!("[files] head {path} failed: {e}");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            info.content_type
                .as_deref()
                .unwrap_or("application/octet-stream"),
        )
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .header(header::ACCEPT_RANGES, "bytes");
    if info.size > 0 {
        builder = builder.header(header::CONTENT_LENGTH, info.size);
    }

    Ok(builder.body(Body::empty()).unwrap())
}

/// DELETE /files/{filename} — delete the single m4a track
pub async fn delete(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(filename): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or((StatusCode::UNAUTHORIZED, "missing token".into()))?;

    if state.config.admin_token.is_empty()
        || token
            .as_bytes()
            .ct_eq(state.config.admin_token.as_bytes())
            .unwrap_u8()
            != 1
    {
        return Err((StatusCode::FORBIDDEN, "invalid token".into()));
    }

    if filename.contains("..") || filename.contains('/') {
        return Err((StatusCode::BAD_REQUEST, "invalid filename".into()));
    }
    let filename = crate::backend::canonical_track_filename(&filename).ok_or((
        StatusCode::BAD_REQUEST,
        "filename must be a soundcloud track".into(),
    ))?;

    let key = crate::backend::key_for(&filename);
    let deleted = state
        .backend
        .delete_file(&key)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("delete: {e}")))?;

    if deleted {
        info!("[files] deleted {filename}");
    } else {
        warn!("[files] {filename} not found for deletion");
    }

    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use bytes::Bytes;
    use futures::StreamExt;

    use super::{ByteStream, ObjectInfo, validate_path, with_first_bytes};

    fn object(chunks: Vec<Result<&'static [u8], &'static str>>) -> (ObjectInfo, ByteStream) {
        let items = chunks
            .into_iter()
            .map(|c| c.map(Bytes::from_static).map_err(std::io::Error::other));
        let info = ObjectInfo {
            size: 6,
            content_type: None,
        };
        (info, Box::pin(futures::stream::iter(items)))
    }

    async fn body(stream: ByteStream) -> Vec<u8> {
        stream.map(|c| c.unwrap().to_vec()).concat().await
    }

    #[tokio::test]
    async fn a_read_that_dies_before_its_first_byte_is_opened_again() {
        let opened = AtomicUsize::new(0);
        let (_, stream) = with_first_bytes(|| {
            let n = opened.fetch_add(1, Ordering::SeqCst);
            async move {
                Ok(match n {
                    0 => object(vec![Err("stalled stream")]),
                    _ => object(vec![Ok(b"abc"), Ok(b"def")]),
                })
            }
        })
        .await
        .unwrap();
        assert_eq!(body(stream).await, b"abcdef");
        assert_eq!(opened.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_read_that_never_gives_a_byte_is_an_error_not_an_empty_answer() {
        let opened = AtomicUsize::new(0);
        let result = with_first_bytes(|| {
            opened.fetch_add(1, Ordering::SeqCst);
            async { Ok(object(vec![Err("stalled stream")])) }
        })
        .await;
        assert!(result.is_err());
        assert_eq!(opened.load(Ordering::SeqCst), super::OPEN_ATTEMPTS);
    }

    #[test]
    fn a_track_object_is_always_read_under_its_canonical_key() {
        for path in ["soundcloud_tracks_42.m4a", "42.m4a"] {
            assert_eq!(
                validate_path(path).unwrap(),
                "soundcloud_tracks_42.m4a",
                "{path}"
            );
        }
        for path in ["covers/42.jpg", "soundcloud_tracks_042.m4a", "notes.txt"] {
            assert_eq!(validate_path(path).unwrap(), path, "{path}");
        }
        assert!(validate_path("../42.m4a").is_err());
        assert!(validate_path("/42.m4a").is_err());
    }
}
