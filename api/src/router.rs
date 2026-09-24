use std::time::Duration;

use axum::Router;
use axum::extract::{MatchedPath, Request, State};
use axum::http::header::{CACHE_CONTROL, PRAGMA, REFERRER_POLICY, VARY};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::Response;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::Instrument;

use crate::modules;
use crate::state::AppState;

pub const MAX_BODY_BYTES: usize = 256 * 1024;

pub fn body_limit() -> axum::extract::DefaultBodyLimit {
    axum::extract::DefaultBodyLimit::max(MAX_BODY_BYTES)
}

pub fn build(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::mirror_request())
        .allow_methods([
            Method::GET,
            Method::HEAD,
            Method::PUT,
            Method::PATCH,
            Method::POST,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers(tower_http::cors::Any)
        .expose_headers([
            HeaderName::from_static("x-session-id"),
            HeaderName::from_static("retry-after"),
            HeaderName::from_static(crate::common::request_id::HEADER),
        ])
        .allow_credentials(false)
        .max_age(Duration::from_secs(3600));

    let http_layer = from_fn_with_state(state.clone(), track_http);
    let premium_layer =
        from_fn_with_state(state.clone(), crate::common::premium_gate::premium_gate);

    let router = Router::new()
        .merge(modules::health::router())
        .merge(modules::admin::router())
        .merge(modules::auth::router(state.admission.clone()))
        .merge(modules::me::router())
        .merge(modules::tracks::router())
        .merge(modules::playlists::router())
        .merge(modules::users::router())
        .merge(modules::resolve::router(state.admission.clone()))
        .merge(modules::history::router())
        .merge(modules::events::router())
        .merge(modules::oauth_apps::router())
        .merge(modules::subscriptions::router())
        .merge(modules::auras::router())
        .merge(modules::likes::router())
        .merge(modules::dislikes::router())
        .merge(modules::featured::router())
        .merge(modules::lyrics::router())
        .merge(modules::collab::router())
        .merge(modules::indexing::router())
        .merge(modules::recommendations::router())
        .merge(modules::enrich::router())
        .merge(modules::artists::router())
        .merge(modules::albums::router())
        .merge(modules::discover::router())
        .merge(modules::discover::admin::router())
        .merge(modules::search::router());

    #[cfg(feature = "profiling")]
    let router = router.merge(crate::profiling::router());

    router
        .with_state(state)
        .layer(body_limit())
        .layer(CompressionLayer::new())
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            Duration::from_secs(60),
        ))
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request| {
                tracing::debug_span!(
                    "http_request",
                    method = %request.method(),
                    path = %request.uri().path()
                )
            }),
        )
        .layer(http_layer)
        .layer(premium_layer)
        .layer(cors)
        .layer(axum::middleware::from_fn(protect_sensitive_responses))
}

pub const CAPABILITY_HEADERS: &[&str] = &["x-session-id", "x-admin-token", "authorization"];

async fn protect_sensitive_responses(req: Request, next: Next) -> Response {
    let sensitive = is_sensitive_path(req.uri().path()) || carries_capability(req.headers());
    let mut response = next.run(req).await;
    if sensitive || hands_out_capability(response.headers()) {
        let headers = response.headers_mut();
        headers.insert(
            CACHE_CONTROL,
            HeaderValue::from_static("private, no-store, max-age=0"),
        );
        headers.insert(PRAGMA, HeaderValue::from_static("no-cache"));
        headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
        headers.insert(
            VARY,
            HeaderValue::from_static("x-session-id, x-admin-token"),
        );
    }
    response
}

fn carries_capability(headers: &HeaderMap) -> bool {
    CAPABILITY_HEADERS
        .iter()
        .any(|name| headers.contains_key(*name))
}

fn hands_out_capability(headers: &HeaderMap) -> bool {
    headers.contains_key("x-session-id")
}

fn is_sensitive_path(path: &str) -> bool {
    let path = path.trim_end_matches('/');
    if path == "/auth" || path.starts_with("/auth/") {
        return true;
    }
    path.starts_with("/tracks/") && path.ends_with("/stream")
}

async fn track_http(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let method = crate::common::http_metrics::method_label(req.method());
    let path = crate::common::http_metrics::route_label(req.extensions().get::<MatchedPath>());
    let request_id = crate::common::request_id::accept_or_mint(
        req.headers()
            .get(crate::common::request_id::HEADER)
            .and_then(|value| value.to_str().ok()),
    );
    let key = format!("{method} {path}");
    let start = std::time::Instant::now();
    let span = tracing::info_span!(
        "request",
        request_id = %request_id,
        method,
        route = %path,
    );
    let mut resp = next.run(req).instrument(span).await;
    let elapsed = start.elapsed();
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        resp.headers_mut().insert(
            HeaderName::from_static(crate::common::request_id::HEADER),
            value,
        );
    }
    state
        .http_metrics
        .record(&key, elapsed.as_millis() as u64, resp.status().as_u16());
    crate::metrics::record_request(method, path, resp.status().as_u16(), elapsed);
    resp
}

#[cfg(test)]
#[path = "router_auth_tests.rs"]
mod router_auth_tests;

#[cfg(test)]
#[path = "router_metrics_tests.rs"]
mod router_metrics_tests;

#[cfg(test)]
#[path = "router_body_tests.rs"]
mod router_body_tests;

#[cfg(test)]
#[path = "router_cache_tests.rs"]
mod router_cache_tests;

#[cfg(test)]
mod tests {
    use super::is_sensitive_path;

    #[test]
    fn stream_path_policy_includes_malformed_trailing_slashes() {
        assert!(is_sensitive_path("/tracks/soundcloud:tracks:42/stream"));
        assert!(is_sensitive_path("/tracks/soundcloud:tracks:42/stream/"));
        assert!(!is_sensitive_path("/tracks/soundcloud:tracks:42"));
    }
}
