use std::borrow::Cow;
use std::time::Duration;

use opentelemetry::trace::Status;
use opentelemetry::{Array, Context, KeyValue, StringValue, Value};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{Span, SpanData, SpanProcessor};

const DROPPED_KEYS: [&str; 16] = [
    "user",
    "user_id",
    "user_urn",
    "connection",
    "connection_id",
    "ip",
    "client_ip",
    "client.address",
    "remote_addr",
    "peer_addr",
    "addr",
    "x-forwarded-for",
    "url.query",
    "url.full",
    "http.url",
    "http.target",
];

const PLAIN_KEYS: [&str; 2] = ["request_id", "scd.origin_trace"];

#[derive(Debug)]
pub(crate) struct Scrubbing<P>(pub(crate) P);

impl<P: SpanProcessor> SpanProcessor for Scrubbing<P> {
    fn on_start(&self, span: &mut Span, cx: &Context) {
        self.0.on_start(span, cx);
    }

    fn on_end(&self, mut span: SpanData) {
        clean(&mut span);
        self.0.on_end(span);
    }

    fn force_flush(&self) -> OTelSdkResult {
        self.0.force_flush()
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.0.shutdown_with_timeout(timeout)
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.0.set_resource(resource);
    }
}

pub(crate) fn clean(span: &mut SpanData) {
    clean_attributes(&mut span.attributes);
    for event in &mut span.events.events {
        clean_attributes(&mut event.attributes);
        if let Some(name) = changed(&event.name) {
            event.name = Cow::Owned(name);
        }
    }
    let description = match &span.status {
        Status::Error { description } => changed(description),
        _ => None,
    };
    if let Some(description) = description {
        span.status = Status::Error {
            description: Cow::Owned(description),
        };
    }
}

fn changed(input: &str) -> Option<String> {
    match redact::text(input) {
        Cow::Owned(cleaned) => Some(cleaned),
        Cow::Borrowed(_) => None,
    }
}

fn clean_attributes(attributes: &mut Vec<KeyValue>) {
    attributes.retain(|attribute| !dropped(attribute.key.as_str()));
    for attribute in attributes.iter_mut() {
        if PLAIN_KEYS.contains(&attribute.key.as_str()) {
            continue;
        }
        match &mut attribute.value {
            Value::String(value) => {
                if let Some(cleaned) = changed(value.as_str()) {
                    *value = StringValue::from(cleaned);
                }
            }
            Value::Array(Array::String(values)) => {
                for value in values.iter_mut() {
                    if let Some(cleaned) = changed(value.as_str()) {
                        *value = StringValue::from(cleaned);
                    }
                }
            }
            _ => {}
        }
    }
}

fn dropped(key: &str) -> bool {
    DROPPED_KEYS.contains(&key) || redact::secret_key(key)
}

#[cfg(test)]
mod tests {
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SimpleSpanProcessor};
    use tracing_subscriber::prelude::*;

    use super::*;

    const CLIENT_ID: &str = "aB3dE5gH7jK9mN1pQ3sT5vX7zC9eF1hJ";
    const OAUTH: &str = "2-291234-123456789-abcDEF0123456";

    fn forbidden(value: &str) -> bool {
        value.contains('?')
            || value.contains(CLIENT_ID)
            || value.contains(OAUTH)
            || value.contains("client_id=")
            || value.contains("secret_token=")
    }

    #[test]
    fn exported_spans_carry_no_query_secret_user_or_connection() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_span_processor(Scrubbing(SimpleSpanProcessor::new(exporter.clone())))
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));
        let uri = format!("https://api-v2.soundcloud.com/tracks/1?client_id={CLIENT_ID}");

        tracing::subscriber::with_default(subscriber, || {
            let span = tracing::info_span!(
                "request",
                error.message = %format!("send failed for uri ({uri})"),
                request_id = "4bf92f3577b3da6a3ce929d0e0e4736f",
                user = "123456",
            );
            let _entered = span.enter();
            tracing::warn!(user = "123456", connection = "abc", url = %uri, "refresh failed");
            tracing::error!(error = %format!("bad {OAUTH}"), "request failed");
        });

        let spans = exporter.get_finished_spans().expect("spans");
        assert_eq!(spans.len(), 1);
        let mut seen = Vec::new();
        for attribute in &spans[0].attributes {
            seen.push((attribute.key.to_string(), attribute.value.to_string()));
        }
        for event in spans[0].events.iter() {
            for attribute in &event.attributes {
                seen.push((attribute.key.to_string(), attribute.value.to_string()));
            }
        }
        for (key, value) in &seen {
            assert!(!forbidden(value), "{key}={value}");
            assert!(!matches!(key.as_str(), "user" | "connection"), "{key}");
        }
        assert!(
            seen.iter()
                .any(|(key, value)| key == "request_id" && value.len() == 32)
        );
        assert!(seen.iter().any(|(key, _)| key == "error.message"));
    }
}
