use opentelemetry::trace::TraceContextExt;
use tracing::Span;
use tracing::field::Empty;
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::traceparent::STATE_KEY;

pub fn peer_call(peer: &'static str, method: &str, route: &'static str) -> Span {
    tracing::info_span!(
        "peer_call",
        otel.name = %format!("{method} {route}"),
        otel.kind = "client",
        otel.status_code = Empty,
        otel.status_description = Empty,
        peer.service = peer,
        server.address = peer,
        http.request.method = method,
        http.route = route,
        http.response.status_code = Empty,
        error.message = Empty,
    )
}

pub fn detached(name: &'static str) -> Span {
    let origin = Span::current().context().span().span_context().clone();
    let span = tracing::info_span!(
        parent: Option::<tracing::Id>::None,
        "detached",
        otel.name = name,
        scd.debug = Empty,
        scd.origin_trace = Empty,
    );
    if origin.is_valid() {
        span.record("scd.origin_trace", origin.trace_id().to_string());
        if origin.trace_state().get(STATE_KEY) == Some("d") {
            span.record("scd.debug", true);
        }
        span.add_link(origin);
    }
    span
}

pub fn answered(span: &Span, status: u16) {
    span.record("http.response.status_code", i64::from(status));
    if status >= 500 {
        span.record("otel.status_code", "ERROR");
    }
}

pub fn broke(span: &Span, message: &str) {
    span.record("otel.status_code", "ERROR");
    span.record("otel.status_description", message);
    span.record("error.message", message);
}

pub fn headers(span: &Span) -> Vec<(&'static str, String)> {
    let context = span.context();
    let span_context = context.span().span_context().clone();
    if !span_context.is_valid() {
        return Vec::new();
    }
    let debug = span_context.trace_state().get(STATE_KEY) == Some("d");
    vec![
        (
            "traceparent",
            format!(
                "00-{}-{}-01",
                span_context.trace_id(),
                span_context.span_id()
            ),
        ),
        (
            "tracestate",
            format!("{STATE_KEY}={}", if debug { "d" } else { "n" }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, Sampler, SdkTracerProvider};
    use tracing_subscriber::prelude::*;

    use crate::request::root_span;

    use super::*;

    fn hop(
        incoming: Option<&str>,
    ) -> (
        Vec<(&'static str, String)>,
        Vec<opentelemetry_sdk::trace::SpanData>,
    ) {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));
        let carried = tracing::subscriber::with_default(subscriber, || {
            let mut first = Request::get("/first");
            if let Some(value) = incoming {
                first = first.header("traceparent", value);
            }
            let first = first.body(Body::empty()).expect("request");
            let root = root_span(&first);
            let carried = root.in_scope(|| {
                let call = peer_call("storage", "GET", "/files/{path}");
                let carried = headers(&call);
                answered(&call, 200);
                carried
            });
            let mut second = Request::get("/second");
            for (name, value) in &carried {
                second = second.header(*name, value);
            }
            let second = second.body(Body::empty()).expect("request");
            drop(root_span(&second));
            drop(root);
            carried
        });
        (carried, exporter.get_finished_spans().expect("spans"))
    }

    fn debug_marked(span: &opentelemetry_sdk::trace::SpanData) -> bool {
        span.attributes
            .iter()
            .any(|attribute| attribute.key.as_str() == "scd.debug")
    }

    #[test]
    fn an_internal_call_joins_the_trace_but_never_makes_it_debug() {
        let (carried, spans) = hop(None);

        let traceparent = &carried[0].1;
        assert!(traceparent.ends_with("-01"));
        assert_eq!(carried[1].1, "scd=n");
        let server = spans
            .iter()
            .find(|s| s.name == "GET unmatched" && s.parent_span_is_remote)
            .expect("downstream root");
        let client = spans
            .iter()
            .find(|s| s.name == "GET /files/{path}")
            .expect("client");
        assert_eq!(
            server.span_context.trace_id(),
            client.span_context.trace_id()
        );
        assert_eq!(server.parent_span_id, client.span_context.span_id());
        assert!(!debug_marked(server));
        assert!(spans.iter().all(|s| !debug_marked(s)));
    }

    #[test]
    fn a_real_debug_request_stays_debug_through_the_hop() {
        let (carried, spans) = hop(Some(
            "00-4bf92f3577b3da6a3ce929d0e0e4736f-00f067aa0ba902b7-01",
        ));

        assert_eq!(carried[1].1, "scd=d");
        let downstream = spans
            .iter()
            .find(|s| {
                s.name == "GET unmatched"
                    && s.parent_span_id
                        == spans
                            .iter()
                            .find(|c| c.name == "GET /files/{path}")
                            .expect("client")
                            .span_context
                            .span_id()
            })
            .expect("downstream root");
        assert!(debug_marked(downstream));
        assert_eq!(
            downstream.span_context.trace_id().to_string(),
            "4bf92f3577b3da6a3ce929d0e0e4736f"
        );
    }

    #[test]
    fn a_client_without_the_debug_bit_keeps_the_whole_chain_quiet() {
        let (carried, spans) = hop(Some(
            "00-4bf92f3577b3da6a3ce929d0e0e4736f-00f067aa0ba902b7-00",
        ));

        assert_eq!(carried[1].1, "scd=n");
        assert!(spans.iter().all(|s| !debug_marked(s)));
    }
}
