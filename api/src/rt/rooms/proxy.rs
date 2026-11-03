use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Request};
use axum::http::StatusCode;
use axum::http::header::{self, HeaderMap, HeaderName, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use serde_json::json;
use url::Url;

pub const TIMEOUT: Duration = Duration::from_secs(60);
pub const BODY_MAX: usize = 1024 * 1024;

const HOP_BY_HOP: [HeaderName; 8] = [
    header::CONNECTION,
    header::PROXY_AUTHENTICATE,
    header::PROXY_AUTHORIZATION,
    header::TE,
    header::TRAILER,
    header::TRANSFER_ENCODING,
    header::UPGRADE,
    header::HOST,
];

pub struct Proxy {
    http: wreq::Client,
    base: String,
}

pub fn router<S>(proxy: Arc<Proxy>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/rooms", any(forward))
        .route("/rooms/{*rest}", any(forward))
        .with_state(proxy)
}

impl Proxy {
    pub fn new(api: &Url) -> Arc<Self> {
        Self::with_timeout(api, TIMEOUT)
    }

    pub fn with_timeout(api: &Url, timeout: Duration) -> Arc<Self> {
        let http = wreq::Client::builder()
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(5))
            .redirect(wreq::redirect::Policy::none())
            .no_gzip()
            .no_brotli()
            .build()
            .expect("the rooms proxy http client builds");
        Arc::new(Self {
            http,
            base: api.as_str().trim_end_matches('/').to_owned(),
        })
    }

    pub fn target(&self, path_and_query: &str) -> String {
        format!("{}{path_and_query}", self.base)
    }
}

async fn forward(
    axum::extract::State(proxy): axum::extract::State<Arc<Proxy>>,
    request: Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let Ok(body) = to_bytes(body, BODY_MAX).await else {
        return refusal(StatusCode::PAYLOAD_TOO_LARGE, "rooms_body_too_large");
    };
    let path_and_query = parts.uri.path_and_query().map_or("/rooms", |p| p.as_str());
    let peer = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0);
    let sent = proxy
        .http
        .request(parts.method.clone(), proxy.target(path_and_query))
        .headers(upstream_headers(&parts.headers, peer))
        .body(body)
        .send()
        .await;
    let upstream = match sent {
        Ok(upstream) => upstream,
        Err(error) if error.is_timeout() => {
            tracing::warn!("rooms upstream timed out");
            return refusal(StatusCode::GATEWAY_TIMEOUT, "rooms_timeout");
        }
        Err(error) => {
            tracing::warn!(
                connect = error.is_connect(),
                "rooms upstream is unreachable"
            );
            return refusal(StatusCode::BAD_GATEWAY, "rooms_unavailable");
        }
    };
    let status = upstream.status();
    let headers = downstream_headers(upstream.headers());
    match upstream.bytes().await {
        Ok(bytes) => {
            let mut response = Response::new(Body::from(bytes));
            *response.status_mut() = status;
            *response.headers_mut() = headers;
            response
        }
        Err(error) => {
            tracing::warn!(
                timeout = error.is_timeout(),
                "rooms upstream broke the body"
            );
            refusal(StatusCode::BAD_GATEWAY, "rooms_unavailable")
        }
    }
}

pub(super) fn upstream_headers(incoming: &HeaderMap, peer: Option<SocketAddr>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in incoming {
        if name != header::CONTENT_LENGTH && !HOP_BY_HOP.contains(name) {
            headers.append(name.clone(), value.clone());
        }
    }
    if !headers.contains_key("x-forwarded-for")
        && let Some(peer) = peer
        && let Ok(value) = HeaderValue::from_str(&peer.ip().to_string())
    {
        headers.insert("x-forwarded-for", value);
    }
    headers
}

fn downstream_headers(upstream: &wreq::header::HeaderMap) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in upstream {
        if name != header::CONTENT_LENGTH && !HOP_BY_HOP.contains(name) {
            headers.append(name.clone(), value.clone());
        }
    }
    headers
}

fn refusal(status: StatusCode, code: &'static str) -> Response {
    (status, Json(json!({ "error": code }))).into_response()
}

#[cfg(test)]
#[path = "proxy_tests.rs"]
mod tests;
