use std::sync::OnceLock;

use opentelemetry::KeyValue;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::SpanExporter;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
use tracing::{Level, Metadata};
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::{EnvFilter, Layer, fmt, prelude::*};

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

    let provider = exporter_provider(service);
    let otel = provider.as_ref().map(|provider| {
        tracing_opentelemetry::layer()
            .with_tracer(provider.tracer(service))
            .with_location(false)
            .with_threads(false)
            .with_target(false)
            .with_tracked_inactivity(false)
            .with_filter(filter_fn(move |meta| ours(meta, own)))
    });

    let registry = tracing_subscriber::registry().with(otel);
    if json {
        registry
            .with(
                fmt::layer()
                    .json()
                    .with_current_span(true)
                    .with_span_list(false)
                    .with_filter(filter),
            )
            .init();
    } else {
        registry
            .with(fmt::layer().with_target(true).with_filter(filter))
            .init();
    }

    Guard(provider)
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
            .with_batch_exporter(exporter)
            .build(),
    )
}

fn ours(meta: &Metadata<'_>, own: &[&str]) -> bool {
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
