use std::sync::OnceLock;

use axum::body::Bytes;
use axum::extract::Query;
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const SIZES: [usize; 3] = [8 * 1024, 64 * 1024, 512 * 1024];
const DEFAULT_SIZE: usize = 64 * 1024;
const DIGEST_HEADER: &str = "x-probe-sha256";

#[derive(Deserialize)]
pub struct Size {
    bytes: Option<usize>,
}

pub async fn probe(Query(size): Query<Size>) -> Response {
    let bytes = size.bytes.unwrap_or(DEFAULT_SIZE);
    let Some(at) = SIZES.iter().position(|allowed| *allowed == bytes) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (
                HeaderName::from_static(DIGEST_HEADER),
                digests()[at].clone(),
            ),
        ],
        Bytes::from_static(&body()[..bytes]),
    )
        .into_response()
}

fn body() -> &'static [u8] {
    static BODY: OnceLock<Vec<u8>> = OnceLock::new();
    BODY.get_or_init(|| fill(SIZES[SIZES.len() - 1]))
}

fn digests() -> &'static [HeaderValue] {
    static DIGESTS: OnceLock<Vec<HeaderValue>> = OnceLock::new();
    DIGESTS.get_or_init(|| {
        SIZES
            .iter()
            .map(|size| {
                let hex: String = Sha256::digest(&body()[..*size])
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                HeaderValue::from_str(&hex).expect("hex is a valid header value")
            })
            .collect()
    })
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
    use sha2::{Digest, Sha256};

    use super::{DIGEST_HEADER, SIZES, Size, probe};

    struct Answer {
        status: StatusCode,
        cache: Option<String>,
        digest: Option<String>,
        body: Vec<u8>,
    }

    async fn ask(bytes: Option<usize>) -> Answer {
        let response = probe(Query(Size { bytes })).await;
        let status = response.status();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        };
        let cache = header(header::CACHE_CONTROL.as_str());
        let digest = header(DIGEST_HEADER);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("the probe body is in memory");
        Answer {
            status,
            cache,
            digest,
            body: body.to_vec(),
        }
    }

    fn hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[tokio::test]
    async fn every_allowed_size_is_served_whole_and_never_cached() {
        let longest = ask(Some(SIZES[2])).await;
        assert_eq!(longest.status, StatusCode::OK);
        assert_eq!(longest.cache.as_deref(), Some("no-store"));
        assert_eq!(longest.body.len(), SIZES[2]);
        for size in SIZES {
            let answer = ask(Some(size)).await;
            assert_eq!(answer.status, StatusCode::OK);
            assert_eq!(answer.body, longest.body[..size]);
        }
        let default = ask(None).await;
        assert_eq!(default.body.len(), 64 * 1024);
        assert!(default.body.iter().any(|byte| *byte != default.body[0]));
    }

    #[tokio::test]
    async fn every_body_carries_its_own_sha256() {
        let mut seen = Vec::new();
        for size in SIZES {
            let answer = ask(Some(size)).await;
            let digest = answer.digest.expect("the digest header is set");
            assert_eq!(digest, hex(&answer.body), "{size}");
            assert!(!seen.contains(&digest));
            seen.push(digest);
        }
        assert_eq!(ask(None).await.digest, Some(seen[1].clone()));
    }

    #[tokio::test]
    async fn any_other_size_is_refused() {
        for size in [0, 1, 65_537, 10 * 1024 * 1024] {
            let answer = ask(Some(size)).await;
            assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{size}");
            assert!(answer.digest.is_none());
            assert!(answer.body.is_empty());
        }
    }
}
