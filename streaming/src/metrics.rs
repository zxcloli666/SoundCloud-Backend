use std::sync::OnceLock;
use std::time::Duration;

use axum::extract::MatchedPath;
use axum::http::Method;
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};

const HTTP_DURATION: &str = "streaming_http_request_duration_seconds";
const HTTP_TOTAL: &str = "streaming_http_requests_total";
const POOL_CONNECTIONS: &str = "streaming_pg_pool_connections";
const POOL_WAIT: &str = "streaming_pg_pool_wait_seconds";
const POOL_WAIT_LAST: &str = "streaming_pg_pool_wait_last_seconds";
const SOURCE_TOTAL: &str = "streaming_source_total";

const HTTP_BUCKETS: &[f64] = &[0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 15.0, 30.0, 60.0, 120.0];
const POOL_WAIT_BUCKETS: &[f64] = &[
    0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 2.5, 5.0, 10.0,
];

pub const UNMATCHED_ROUTE: &str = "unmatched";

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

pub fn init() {
    if HANDLE.get().is_some() {
        return;
    }
    let builder = PrometheusBuilder::new()
        .set_buckets_for_metric(Matcher::Full(HTTP_DURATION.to_owned()), HTTP_BUCKETS)
        .and_then(|builder| {
            builder.set_buckets_for_metric(Matcher::Full(POOL_WAIT.to_owned()), POOL_WAIT_BUCKETS)
        });
    let builder = match builder {
        Ok(builder) => builder,
        Err(error) => {
            tracing::warn!(%error, "metrics buckets rejected");
            return;
        }
    };
    match builder.install_recorder() {
        Ok(handle) => {
            let _ = HANDLE.set(handle);
        }
        Err(error) => tracing::warn!(%error, "metrics recorder unavailable"),
    }
}

pub fn route_label(matched: Option<&MatchedPath>) -> String {
    match matched.map(MatchedPath::as_str) {
        Some(pattern) if !pattern.is_empty() => pattern.to_owned(),
        _ => UNMATCHED_ROUTE.to_owned(),
    }
}

pub fn method_label(method: &Method) -> &'static str {
    match *method {
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::POST => "POST",
        Method::DELETE => "DELETE",
        Method::OPTIONS => "OPTIONS",
        _ => "other",
    }
}

pub fn status_label(status: u16) -> &'static str {
    match status {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        500..=599 => "5xx",
        _ => "other",
    }
}

pub fn record_request(route: String, method: &'static str, status: u16, elapsed: Duration) {
    if HANDLE.get().is_none() {
        return;
    }
    let class = status_label(status);
    metrics::histogram!(HTTP_DURATION, "route" => route.clone(), "method" => method)
        .record(elapsed.as_secs_f64());
    metrics::counter!(HTTP_TOTAL, "route" => route, "method" => method, "status" => class)
        .increment(1);
}

pub fn record_source(source: &'static str, outcome: &'static str) {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::counter!(SOURCE_TOTAL, "source" => source, "outcome" => outcome).increment(1);
}

pub fn record_pool_wait(outcome: &'static str, waited: Duration) {
    if HANDLE.get().is_none() {
        return;
    }
    let seconds = waited.as_secs_f64();
    metrics::histogram!(POOL_WAIT, "outcome" => outcome).record(seconds);
    metrics::gauge!(POOL_WAIT_LAST).set(seconds);
}

pub fn record_pool_state(max: usize, size: usize, available: usize, waiting: usize) {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::gauge!(POOL_CONNECTIONS, "state" => "max").set(max as f64);
    metrics::gauge!(POOL_CONNECTIONS, "state" => "open").set(size as f64);
    metrics::gauge!(POOL_CONNECTIONS, "state" => "idle").set(available as f64);
    metrics::gauge!(POOL_CONNECTIONS, "state" => "busy").set(size.saturating_sub(available) as f64);
    metrics::gauge!(POOL_CONNECTIONS, "state" => "waiting").set(waiting as f64);
}

pub fn render() -> Option<String> {
    Some(HANDLE.get()?.render())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_route_nobody_matched_collapses_into_one_label() {
        assert_eq!(route_label(None), UNMATCHED_ROUTE);
    }

    #[test]
    fn a_method_nobody_serves_collapses_into_one_label() {
        let invented = Method::from_bytes(b"BREW").expect("a token is a valid method");
        assert_eq!(method_label(&invented), "other");
        assert_eq!(method_label(&Method::GET), "GET");
        assert_eq!(method_label(&Method::POST), "POST");
    }

    #[test]
    fn statuses_collapse_into_five_classes_and_never_into_a_number() {
        assert_eq!(status_label(200), "2xx");
        assert_eq!(status_label(206), "2xx");
        assert_eq!(status_label(404), "4xx");
        assert_eq!(status_label(503), "5xx");
        assert_eq!(status_label(700), "other");
    }
}
