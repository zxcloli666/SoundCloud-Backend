use std::sync::OnceLock;

use axum::body::Bytes;
use axum::extract::Query;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

const SIZES: [usize; 3] = [8 * 1024, 64 * 1024, 512 * 1024];
const DEFAULT_SIZE: usize = 64 * 1024;

#[derive(Deserialize)]
pub struct Size {
    bytes: Option<usize>,
}

pub async fn probe(Query(size): Query<Size>) -> Response {
    let bytes = size.bytes.unwrap_or(DEFAULT_SIZE);
    if !SIZES.contains(&bytes) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    (
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Bytes::from_static(&body()[..bytes]),
    )
        .into_response()
}

fn body() -> &'static [u8] {
    static BODY: OnceLock<Vec<u8>> = OnceLock::new();
    BODY.get_or_init(|| fill(SIZES[SIZES.len() - 1]))
}

fn fill(size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(size);
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    while out.len() < size {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.extend_from_slice(&state.to_le_bytes());
    }
    out.truncate(size);
    out
}

#[cfg(test)]
mod tests {
    use axum::extract::Query;
    use axum::http::{StatusCode, header};

    use super::{SIZES, Size, probe};

    async fn ask(bytes: Option<usize>) -> (StatusCode, Option<String>, Vec<u8>) {
        let response = probe(Query(Size { bytes })).await;
        let status = response.status();
        let cache = response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the probe body is in memory");
        (status, cache, body.to_vec())
    }

    #[tokio::test]
    async fn every_allowed_size_is_served_whole_and_never_cached() {
        let (status, cache, longest) = ask(Some(SIZES[2])).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(cache.as_deref(), Some("no-store"));
        assert_eq!(longest.len(), SIZES[2]);
        for size in SIZES {
            let (status, _, body) = ask(Some(size)).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body, longest[..size]);
        }
        let (_, _, default) = ask(None).await;
        assert_eq!(default.len(), 64 * 1024);
        assert!(default.iter().any(|byte| *byte != default[0]));
    }

    #[tokio::test]
    async fn any_other_size_is_refused() {
        for size in [0, 1, 65_537, 10 * 1024 * 1024] {
            let (status, _, body) = ask(Some(size)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{size}");
            assert!(body.is_empty());
        }
    }
}
