use axum::extract::{MatchedPath, Request};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;
use opentelemetry::trace::TraceContextExt;
use tracing::field::Empty;
use tracing::{Instrument, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::init::HOST;
use crate::traceparent;

pub fn root_span(req: &Request) -> Span {
    let method = req.method().as_str();
    let route = req
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
        .unwrap_or("unmatched");
    let name = format!("{method} {route}");
    let host = HOST.get().map(String::as_str).unwrap_or("");
    let span = tracing::info_span!(
        "request",
        otel.name = %name,
        otel.kind = "server",
        otel.status_code = Empty,
        otel.status_description = Empty,
        http.request.method = method,
        http.route = route,
        http.response.status_code = Empty,
        scd.debug = Empty,
        scd.host = host,
        scd.outcome = Empty,
        error.message = Empty,
        request_id = Empty,
    );
    if let Some(incoming) = incoming(req.headers()) {
        if incoming.debug() {
            span.record("scd.debug", true);
        }
        let _ = span.set_parent(incoming.context());
    }
    span
}

fn incoming(headers: &HeaderMap) -> Option<traceparent::Incoming> {
    let value = headers.get("traceparent")?.to_str().ok()?;
    traceparent::parse(value)
}

pub fn trace_id(span: &Span, headers: &HeaderMap) -> Option<String> {
    let context = span.context();
    let span_context = context.span().span_context().clone();
    if span_context.is_valid() {
        return Some(span_context.trace_id().to_string());
    }
    incoming(headers).map(|incoming| incoming.trace.to_string())
}

pub fn finish(span: &Span, status: u16) {
    span.record("http.response.status_code", i64::from(status));
    if status >= 500 {
        span.record("otel.status_code", "ERROR");
    }
}

pub fn fail(span: &Span, message: &str) {
    span.record("otel.status_code", "ERROR");
    span.record("otel.status_description", message);
    span.record("error.message", message);
}

pub async fn trace_requests(req: Request, next: Next) -> Response {
    let span = root_span(&req);
    let response = next.run(req).instrument(span.clone()).await;
    finish(&span, response.status().as_u16());
    response
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;

    use super::*;

    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn app() -> Router {
        Router::new()
            .route("/ok/{id}", get(|| async { "fine" }))
            .route("/boom", get(|| async { StatusCode::BAD_GATEWAY }))
            .layer(axum::middleware::from_fn(trace_requests))
    }

    #[test]
    fn requests_pass_through_the_root_span_untouched() {
        let _serial = SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let ok = runtime
            .block_on(
                app().oneshot(
                    HttpRequest::get("/ok/7")
                        .body(Body::empty())
                        .expect("request"),
                ),
            )
            .expect("response");
        let boom = runtime
            .block_on(
                app().oneshot(
                    HttpRequest::get("/boom")
                        .body(Body::empty())
                        .expect("request"),
                ),
            )
            .expect("response");

        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(boom.status(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn the_trace_id_comes_from_the_header_when_no_exporter_runs() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "traceparent",
            "00-4bf92f3577b3da6a3ce929d0e0e4736f-00f067aa0ba902b7-00"
                .parse()
                .expect("header"),
        );

        let id = trace_id(&Span::none(), &headers);

        assert_eq!(id.as_deref(), Some("4bf92f3577b3da6a3ce929d0e0e4736f"));
        assert_eq!(trace_id(&Span::none(), &HeaderMap::new()), None);
    }

    #[test]
    fn the_root_span_joins_the_incoming_trace_and_marks_failures() {
        let _serial = SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        use opentelemetry::trace::TracerProvider;
        use opentelemetry_sdk::trace::{InMemorySpanExporter, Sampler, SdkTracerProvider};
        use tracing_subscriber::prelude::*;

        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));

        tracing::subscriber::with_default(subscriber, || {
            let request = HttpRequest::get("/boom")
                .header(
                    "traceparent",
                    "00-4bf92f3577b3da6a3ce929d0e0e4736f-00f067aa0ba902b7-01",
                )
                .body(Body::empty())
                .expect("request");
            let span = root_span(&request);
            let id = trace_id(&span, request.headers());
            fail(&span, "upstream said no");
            finish(&span, 502);
            drop(span);
            assert_eq!(id.as_deref(), Some("4bf92f3577b3da6a3ce929d0e0e4736f"));
        });

        let spans = exporter.get_finished_spans().expect("spans");
        let root = &spans[0];
        assert_eq!(
            root.span_context.trace_id().to_string(),
            "4bf92f3577b3da6a3ce929d0e0e4736f"
        );
        assert_eq!(root.parent_span_id.to_string(), "00f067aa0ba902b7");
        assert_eq!(root.name, "GET unmatched");
        assert!(matches!(
            root.status,
            opentelemetry::trace::Status::Error { .. }
        ));
        let has = |key: &str| root.attributes.iter().any(|kv| kv.key.as_str() == key);
        assert!(has("scd.debug"));
        assert!(has("http.response.status_code"));
        assert!(has("error.message"));
    }
}
