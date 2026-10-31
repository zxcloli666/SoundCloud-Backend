use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use opentelemetry::KeyValue;
use opentelemetry::trace::{Span as _, SpanKind, Status, TraceContextExt, Tracer};
use opentelemetry_sdk::trace::SdkTracer;
use tracing::dispatcher::WeakDispatch;
use tracing::field::{Field, Visit};
use tracing::{Event, Span, Subscriber};
use tracing_opentelemetry::get_otel_context;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::{LookupSpan, Registry};

pub(crate) const TARGET: &str = "telemetry::dependency";
pub(crate) const SQLX: &str = "sqlx::query";

const SLOW: Duration = Duration::from_millis(20);
const FIRST: usize = 2;
const CEILING: usize = 12;

pub fn call(system: &'static str, operation: &str, elapsed: Duration, failed: bool) {
    tracing::debug!(
        target: "telemetry::dependency",
        system,
        operation,
        elapsed_us = elapsed.as_micros() as u64,
        failed,
    );
}

pub fn statement(sql: &str, rows: u64, elapsed: Duration) {
    tracing::debug!(
        target: "sqlx::query",
        summary = "",
        db.statement = sql,
        rows_returned = rows,
        rows_affected = 0u64,
        elapsed_secs = elapsed.as_secs_f64(),
    );
}

#[derive(Default)]
struct Counter {
    calls: AtomicU64,
    rows: AtomicU64,
    nanos: AtomicU64,
}

impl Counter {
    fn add(&self, rows: u64, elapsed: Duration) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.rows.fetch_add(rows, Ordering::Relaxed);
        self.nanos
            .fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
    }

    fn read(&self) -> (u64, u64, u64) {
        (
            self.calls.load(Ordering::Relaxed),
            self.rows.load(Ordering::Relaxed),
            self.nanos.load(Ordering::Relaxed) / 1_000_000,
        )
    }
}

pub(crate) struct Totals {
    dispatch: WeakDispatch,
    postgres: Counter,
    redis: Counter,
    qdrant: Counter,
    exported: AtomicUsize,
}

impl Totals {
    fn counter(&self, system: &str) -> Option<&Counter> {
        match system {
            "postgresql" => Some(&self.postgres),
            "redis" => Some(&self.redis),
            "qdrant" => Some(&self.qdrant),
            _ => None,
        }
    }

    fn wants_span(&self, elapsed: Duration, failed: bool) -> bool {
        let taken = self.exported.load(Ordering::Relaxed);
        let wanted = ((failed || elapsed >= SLOW) && taken < CEILING) || taken < FIRST;
        if wanted {
            self.exported.fetch_add(1, Ordering::Relaxed);
        }
        wanted
    }
}

pub(crate) fn attach(span: &Span) {
    span.with_subscriber(|(id, dispatch)| {
        let registry = dispatch.downcast_ref::<Registry>()?;
        let data = registry.span(id)?;
        data.extensions_mut().insert(Arc::new(Totals {
            dispatch: dispatch.downgrade(),
            postgres: Counter::default(),
            redis: Counter::default(),
            qdrant: Counter::default(),
            exported: AtomicUsize::new(0),
        }));
        Some(())
    });
}

fn totals_of(span: &Span) -> Option<Arc<Totals>> {
    span.with_subscriber(|(id, dispatch)| {
        let registry = dispatch.downcast_ref::<Registry>()?;
        let data = registry.span(id)?;
        data.extensions().get::<Arc<Totals>>().cloned()
    })
    .flatten()
}

pub(crate) fn flush(span: &Span) {
    let Some(totals) = totals_of(span) else {
        return;
    };
    let (queries, rows, millis) = totals.postgres.read();
    if queries > 0 {
        span.record("db.queries", queries as i64);
        span.record("db.rows", rows as i64);
        span.record("db.time_ms", millis as i64);
    }
    let (calls, _, millis) = totals.redis.read();
    if calls > 0 {
        span.record("redis.calls", calls as i64);
        span.record("redis.time_ms", millis as i64);
    }
    let (calls, _, millis) = totals.qdrant.read();
    if calls > 0 {
        span.record("qdrant.calls", calls as i64);
        span.record("qdrant.time_ms", millis as i64);
    }
}

#[derive(Default)]
struct Fields {
    system: Option<String>,
    operation: Option<String>,
    summary: Option<String>,
    statement: Option<String>,
    rows: u64,
    elapsed: Option<Duration>,
    failed: bool,
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "system" => self.system = Some(value.to_owned()),
            "operation" => self.operation = Some(value.to_owned()),
            "summary" => self.summary = Some(value.to_owned()),
            "db.statement" => self.statement = Some(value.to_owned()),
            _ => {}
        }
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        match field.name() {
            "rows_returned" | "rows_affected" => self.rows = self.rows.max(value),
            "elapsed_us" => self.elapsed = Some(Duration::from_micros(value)),
            _ => {}
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        if let Ok(value) = u64::try_from(value) {
            self.record_u64(field, value);
        }
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        if field.name() == "elapsed_secs" && value.is_finite() && value >= 0.0 {
            self.elapsed = Some(Duration::from_secs_f64(value));
        }
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        if field.name() == "failed" {
            self.failed = value;
        }
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn std::fmt::Debug) {}
}

struct Finished {
    system: &'static str,
    name: String,
    operation: String,
    collection: Option<String>,
    hash: Option<String>,
    rows: u64,
    elapsed: Duration,
    failed: bool,
}

fn finished(target: &str, fields: Fields) -> Option<Finished> {
    let elapsed = fields.elapsed?;
    if target == SQLX {
        let text = match fields.statement.as_deref().map(str::trim) {
            Some(statement) if !statement.is_empty() => statement,
            _ => fields.summary.as_deref()?,
        };
        let described = describe(text);
        let name = match &described.table {
            Some(table) => format!("{} {table}", described.operation),
            None => described.operation.clone(),
        };
        return Some(Finished {
            system: "postgresql",
            name,
            operation: described.operation,
            collection: described.table,
            hash: Some(described.hash),
            rows: fields.rows,
            elapsed,
            failed: false,
        });
    }
    let system = match fields.system.as_deref()? {
        "redis" => "redis",
        "qdrant" => "qdrant",
        "postgres" | "postgresql" => "postgresql",
        _ => "other",
    };
    let operation = fields.operation.unwrap_or_default();
    Some(Finished {
        system,
        name: format!("{system} {operation}"),
        operation,
        collection: None,
        hash: None,
        rows: 0,
        elapsed,
        failed: fields.failed,
    })
}

struct Described {
    operation: String,
    table: Option<String>,
    hash: String,
}

fn describe(text: &str) -> Described {
    let operation = text
        .split_whitespace()
        .next()
        .map(|word| {
            word.chars()
                .filter(char::is_ascii_alphabetic)
                .take(12)
                .collect::<String>()
                .to_ascii_uppercase()
        })
        .filter(|word| !word.is_empty())
        .unwrap_or_else(|| "QUERY".to_owned());
    let words: Vec<&str> = text
        .split_whitespace()
        .map(|word| word.trim_matches(|c| matches!(c, '(' | ')' | ',' | ';' | '"')))
        .collect();
    let table = words
        .iter()
        .enumerate()
        .find(|(_, word)| {
            ["FROM", "INTO", "UPDATE", "JOIN"]
                .iter()
                .any(|keyword| word.eq_ignore_ascii_case(keyword))
        })
        .and_then(|(at, _)| words.get(at + 1).copied())
        .map(|word| word.rsplit('.').next().unwrap_or(word).to_ascii_lowercase())
        .filter(|word| {
            word.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !["select", "only", "lateral", "unnest", "set"].contains(&word.as_str())
        });
    Described {
        operation,
        table,
        hash: hash(text),
    }
}

fn hash(text: &str) -> String {
    let mut state: u64 = 0xcbf29ce484222325;
    for word in text.split_whitespace() {
        for byte in word.bytes().chain(std::iter::once(b' ')) {
            state ^= u64::from(byte);
            state = state.wrapping_mul(0x100000001b3);
        }
    }
    format!("{state:016x}")
}

pub(crate) struct Dependencies {
    pub(crate) tracer: SdkTracer,
}

impl<S> Layer<S> for Dependencies
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        let Some(call) = finished(event.metadata().target(), fields) else {
            return;
        };
        let Some(scope) = ctx.event_scope(event) else {
            return;
        };
        let mut nearest = None;
        let mut totals = None;
        for span in scope {
            if nearest.is_none() {
                nearest = Some(span.id());
            }
            let found = span.extensions().get::<Arc<Totals>>().cloned();
            if found.is_some() {
                totals = found;
                break;
            }
        }
        let (Some(totals), Some(nearest)) = (totals, nearest) else {
            return;
        };
        if let Some(counter) = totals.counter(call.system) {
            counter.add(call.rows, call.elapsed);
        }
        if !totals.wants_span(call.elapsed, call.failed) {
            return;
        }
        let Some(dispatch) = totals.dispatch.upgrade() else {
            return;
        };
        let Some(parent) = get_otel_context(&nearest, &dispatch) else {
            return;
        };
        if !parent.span().span_context().is_valid() {
            return;
        }
        self.export(&parent, &call);
    }
}

impl Dependencies {
    fn export(&self, parent: &opentelemetry::Context, call: &Finished) {
        let now = SystemTime::now();
        let started = now.checked_sub(call.elapsed).unwrap_or(now);
        let mut attributes = vec![
            KeyValue::new("db.system.name", call.system),
            KeyValue::new("peer.service", peer(call.system)),
            KeyValue::new("db.operation.name", call.operation.clone()),
        ];
        if let Some(collection) = &call.collection {
            attributes.push(KeyValue::new("db.collection.name", collection.clone()));
        }
        if let Some(hash) = &call.hash {
            attributes.push(KeyValue::new("db.query.hash", hash.clone()));
            attributes.push(KeyValue::new(
                "db.response.returned_rows",
                i64::try_from(call.rows).unwrap_or(i64::MAX),
            ));
        }
        let mut builder = self
            .tracer
            .span_builder(call.name.clone())
            .with_kind(SpanKind::Client)
            .with_start_time(started)
            .with_attributes(attributes);
        if call.failed {
            builder = builder.with_status(Status::error("dependency call failed"));
        }
        let mut span = self.tracer.build_with_context(builder, parent);
        span.end_with_timestamp(now);
    }
}

fn peer(system: &str) -> &'static str {
    match system {
        "postgresql" => "postgres",
        "redis" => "redis",
        "qdrant" => "qdrant",
        _ => "dependency",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_statement_is_named_by_its_verb_and_table_and_never_by_its_values() {
        let described = describe(
            "\n\nSELECT t.id, t.title\nFROM public.Tracks t\nJOIN artists a ON a.id = t.artist_id\nWHERE t.id = $1\n",
        );

        assert_eq!(described.operation, "SELECT");
        assert_eq!(described.table.as_deref(), Some("tracks"));
        assert_eq!(described.hash.len(), 16);
    }

    #[test]
    fn writes_and_oddities_still_get_a_name() {
        assert_eq!(
            describe("INSERT INTO likes (user_id) VALUES ($1) ON CONFLICT DO UPDATE SET a = 1")
                .table
                .as_deref(),
            Some("likes")
        );
        assert_eq!(
            describe("DELETE FROM sessions WHERE id = $1")
                .table
                .as_deref(),
            Some("sessions")
        );
        assert_eq!(describe("SELECT set_config('a', '1', true)").table, None);
        assert_eq!(describe("").operation, "QUERY");
    }

    #[test]
    fn the_same_statement_hashes_the_same_across_whitespace() {
        assert_eq!(hash("SELECT  1\nFROM x"), hash("SELECT 1 FROM x"));
        assert_ne!(hash("SELECT 1 FROM x"), hash("SELECT 2 FROM x"));
    }
}
