use tracing::Span;
use tracing::field::Empty;

use crate::error::{ScError, ScResult};

pub(crate) fn upstream(name: &str, route: &'static str) -> Span {
    tracing::info_span!(
        "upstream",
        otel.name = %format!("upstream {name}"),
        otel.kind = "client",
        otel.status_code = Empty,
        otel.status_description = Empty,
        peer.service = "soundcloud",
        scd.upstream = name,
        scd.route = route,
        scd.outcome = Empty,
        http.response.status_code = Empty,
        error_type = Empty,
        error.message = Empty,
    )
}

pub(crate) fn record<T>(span: &Span, result: &ScResult<T>) {
    let Err(error) = result else {
        span.record("scd.outcome", "ok");
        return;
    };
    span.record("scd.outcome", "error");
    span.record("error_type", error.kind());
    let message = error.to_string();
    if let Some(status) = error.status() {
        span.record("http.response.status_code", i64::from(status));
    }
    let failed = match error {
        ScError::Api { status, .. } => *status >= 500 || *status == 429,
        _ => true,
    };
    if failed {
        span.record("otel.status_code", "ERROR");
        span.record("otel.status_description", message.as_str());
    }
    span.record("error.message", message.as_str());
}
