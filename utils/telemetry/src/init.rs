use std::sync::OnceLock;

use opentelemetry::KeyValue;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::SpanExporter;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{BatchSpanProcessor, Sampler, SdkTracer, SdkTracerProvider};
use tracing::{Level, Metadata, Subscriber};
use tracing_subscriber::filter::{LevelFilter, filter_fn};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{EnvFilter, Layer, fmt, prelude::*};

use crate::dependency::{self, Dependencies};
use crate::scrub::Scrubbing;
use crate::uncached::Uncached;

pub(crate) static HOST: OnceLock<String> = OnceLock::new();

pub struct Guard(Option<SdkTracerProvider>);

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(provider) = self.0.take() {
            let _ = provider.shutdown();
        }
    }
}

pub fn init(service: &'static str, default_filter: &str, own: &'static [&'static str]) -> Guard {
    let _ = HOST.set(std::env::var("SCD_HOST").unwrap_or_default());

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));
    let json = std::env::var("LOG_FORMAT")
        .map(|v| v.eq_ignore_ascii_case("json"))
        .unwrap_or(false);

    let scrubbed_stdout = || redact::Scrubbing::new(std::io::stdout());
    let provider = exporter_provider(service);
    let otel = provider
        .as_ref()
        .map(|provider| otel_layer(provider.tracer(service), own));
    let dependencies = provider
        .as_ref()
        .map(|provider| dependency_layer(provider.tracer(service), own));

    tracing_subscriber::registry()
        .with(otel)
        .with(dependencies)
        .with(log_layer(filter, json, scrubbed_stdout))
        .init();

    Guard(provider)
}

pub(crate) fn otel_layer<S>(tracer: SdkTracer, own: &'static [&'static str]) -> impl Layer<S>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    tracing_opentelemetry::layer()
        .with_tracer(tracer)
        .with_location(false)
        .with_threads(false)
        .with_target(false)
        .with_tracked_inactivity(false)
        .with_filter(Uncached(filter_fn(move |meta| ours(meta, own))))
}

pub(crate) fn dependency_layer<S>(tracer: SdkTracer, own: &'static [&'static str]) -> impl Layer<S>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    Dependencies { tracer }.with_filter(Uncached(
        filter_fn(move |meta| {
            if meta.is_span() {
                ours(meta, own)
            } else {
                matches!(meta.target(), dependency::SQLX | dependency::TARGET)
            }
        })
        .with_max_level_hint(LevelFilter::DEBUG),
    ))
}

fn log_layer<S, W>(filter: EnvFilter, json: bool, writer: W) -> Box<dyn Layer<S> + Send + Sync>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    if json {
        fmt::layer()
            .json()
            .with_writer(writer)
            .with_current_span(true)
            .with_span_list(false)
            .with_filter(Uncached(filter))
            .boxed()
    } else {
        fmt::layer()
            .with_target(true)
            .with_writer(writer)
            .with_filter(Uncached(filter))
            .boxed()
    }
}

fn exporter_provider(service: &'static str) -> Option<SdkTracerProvider> {
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok()?;
    if endpoint.trim().is_empty() {
        return None;
    }
    let exporter = match SpanExporter::builder().with_tonic().build() {
        Ok(exporter) => exporter,
        Err(error) => {
            eprintln!("otel exporter disabled: {error}");
            return None;
        }
    };
    let host = HOST.get().cloned().unwrap_or_default();
    let service_name = if host.is_empty() {
        service.to_owned()
    } else {
        format!("{service}-{host}")
    };
    let resource = Resource::builder()
        .with_service_name(service_name)
        .with_attribute(KeyValue::new("scd.host", host))
        .build();
    Some(
        SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .with_resource(resource)
            .with_span_processor(Scrubbing(BatchSpanProcessor::builder(exporter).build()))
            .build(),
    )
}

fn ours(meta: &Metadata<'_>, own: &[&str]) -> bool {
    if meta.is_event() && meta.target() == "scd_event" {
        return *meta.level() <= Level::DEBUG;
    }
    let target = meta.target().split("::").next().unwrap_or("");
    if !own.contains(&target) {
        return false;
    }
    if meta.is_span() {
        *meta.level() <= Level::INFO
    } else {
        *meta.level() <= Level::WARN
    }
}

#[cfg(test)]
mod tests {
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

    use axum::body::Body;
    use axum::http::Request;
    use opentelemetry::Value;
    use opentelemetry::trace::SpanKind;
    use opentelemetry_sdk::trace::Sampler;
    use std::time::Duration;

    use super::{dependency_layer, log_layer, otel_layer};
    use crate::request::{finish, root_span};

    #[test]
    fn a_bare_log_level_check_does_not_hide_the_next_span() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(Some(otel_layer(provider.tracer("test"), &["telemetry"])))
            .with(log_layer(
                EnvFilter::new("info,sqlx=warn"),
                false,
                std::io::sink,
            ));
        let sqlx_check = log::Metadata::builder()
            .level(log::Level::Debug)
            .target("sqlx::query")
            .build();

        tracing::subscriber::with_default(subscriber, || {
            for _ in 0..10 {
                log::Log::enabled(&tracing_log::LogTracer::new(), &sqlx_check);
                drop(tracing::info_span!("request"));
            }
        });

        assert_eq!(exporter.get_finished_spans().expect("spans").len(), 10);
    }

    fn attribute(span: &opentelemetry_sdk::trace::SpanData, key: &str) -> Option<Value> {
        span.attributes
            .iter()
            .find(|attribute| attribute.key.as_str() == key)
            .map(|attribute| attribute.value.clone())
    }

    #[test]
    fn queries_and_cache_calls_become_few_child_spans_and_counters_on_the_request() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_sampler(Sampler::AlwaysOn)
            .with_simple_exporter(exporter.clone())
            .build();
        let own: &'static [&'static str] = &["telemetry"];
        let subscriber = tracing_subscriber::registry()
            .with(otel_layer(provider.tracer("test"), own))
            .with(dependency_layer(provider.tracer("test"), own))
            .with(log_layer(
                EnvFilter::new("info,sqlx=warn"),
                false,
                std::io::sink,
            ));

        tracing::subscriber::with_default(subscriber, || {
            let request = Request::get("/tracks/1")
                .body(Body::empty())
                .expect("request");
            let root = root_span(&request);
            root.in_scope(|| {
                for _ in 0..5 {
                    tracing::debug!(
                        target: "sqlx::query",
                        summary = "SELECT id FROM tracks",
                        db.statement = "\n\nSELECT id FROM tracks WHERE id = $1\n",
                        rows_affected = 0u64,
                        rows_returned = 1u64,
                        elapsed_secs = 0.001f64,
                    );
                }
                tracing::warn!(
                    target: "sqlx::query",
                    summary = "SELECT id FROM tracks",
                    db.statement = "\n\nSELECT id FROM tracks WHERE id = $1\n",
                    rows_affected = 0u64,
                    rows_returned = 1u64,
                    elapsed_secs = 0.05f64,
                );
                crate::dependency::call("redis", "get", Duration::from_millis(1), false);
                crate::dependency::call("redis", "set", Duration::from_millis(40), true);
            });
            finish(&root, 200);
        });

        let spans = exporter.get_finished_spans().expect("spans");
        let root = spans
            .iter()
            .find(|s| s.name == "GET unmatched")
            .expect("root");
        let children: Vec<_> = spans
            .iter()
            .filter(|s| s.parent_span_id == root.span_context.span_id())
            .collect();
        assert_eq!(
            children.len(),
            4,
            "{:?}",
            children
                .iter()
                .map(|s| s.name.to_string())
                .collect::<Vec<_>>()
        );
        assert!(children.iter().all(|s| s.span_kind == SpanKind::Client));
        assert_eq!(
            children
                .iter()
                .filter(|s| s.name == "SELECT tracks")
                .count(),
            3
        );
        let failed = children
            .iter()
            .find(|s| s.name == "redis set")
            .expect("redis set");
        assert!(matches!(
            failed.status,
            opentelemetry::trace::Status::Error { .. }
        ));
        assert_eq!(attribute(root, "db.queries"), Some(Value::I64(6)));
        assert_eq!(attribute(root, "db.rows"), Some(Value::I64(6)));
        assert_eq!(attribute(root, "redis.calls"), Some(Value::I64(2)));
        for span in &children {
            for attribute in &span.attributes {
                let shown = attribute.value.to_string();
                assert!(!shown.contains("WHERE") && !shown.contains("$1"), "{shown}");
            }
        }
    }

    #[derive(Clone, Default)]
    struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("sink").extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn log_lines_never_carry_query_strings_secrets_or_ids() {
        const CLIENT_ID: &str = "aB3dE5gH7jK9mN1pQ3sT5vX7zC9eF1hJ";
        for json in [false, true] {
            let sink = Sink::default();
            let writer = {
                let sink = sink.clone();
                move || redact::Scrubbing::new(sink.clone())
            };
            let subscriber = tracing_subscriber::registry().with(log_layer(
                EnvFilter::new("info"),
                json,
                writer,
            ));

            tracing::subscriber::with_default(subscriber, || {
                let uri = format!("https://api-v2.soundcloud.com/tracks/1?client_id={CLIENT_ID}");
                tracing::warn!(url = %uri, client_id = CLIENT_ID, "send failed for uri ({uri})");
            });

            let written = String::from_utf8(sink.0.lock().expect("sink").clone()).expect("utf8");
            assert!(written.contains("send failed"), "{written}");
            assert!(!written.contains(CLIENT_ID), "{written}");
            assert!(!written.contains('?'), "{written}");
        }
    }
}
