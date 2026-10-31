use std::future::Future;

use tracing::field::Empty;
use tracing::{Instrument, Span};

use super::BackendError;

pub(super) async fn s3<T>(
    op: &'static str,
    attempt: impl Future<Output = Result<T, BackendError>>,
) -> Result<T, BackendError> {
    let span = tracing::info_span!(
        "s3",
        otel.name = %format!("s3 {op}"),
        otel.kind = "client",
        peer.service = "s3",
        otel.status_code = Empty,
        otel.status_description = Empty,
        scd.status = Empty,
        bytes = Empty,
        error.message = Empty,
    );
    let result = attempt.instrument(span.clone()).await;
    match &result {
        Ok(_) => {
            span.record("scd.status", "ok");
        }
        Err(BackendError::NotFound) => {
            span.record("scd.status", "not_found");
        }
        Err(error) => fail(&span, &error.to_string()),
    }
    result
}

pub(super) fn bytes(size: u64) {
    Span::current().record("bytes", size);
}

fn fail(span: &Span, message: &str) {
    span.record("scd.status", "error");
    span.record("otel.status_code", "ERROR");
    span.record("otel.status_description", message);
    span.record("error.message", message);
}
