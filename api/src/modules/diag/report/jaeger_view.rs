use serde_json::{Map, Value};

use super::super::jaeger::{SpanView, TraceView};

pub fn trace(id: &str, spans: Vec<SpanView>) -> TraceView {
    TraceView {
        trace: id.to_owned(),
        spans,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn span(
    service: &str,
    host: &str,
    name: &str,
    id: &str,
    parent: Option<&str>,
    start_after_ms: i64,
    ms: u64,
    error: bool,
    attrs: &[(&str, &str)],
) -> SpanView {
    let base = chrono::DateTime::parse_from_rfc3339("2026-10-29T10:00:00.000Z")
        .expect("time")
        .timestamp_millis();
    let start = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(base + start_after_ms)
        .expect("time")
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut map = Map::new();
    for (key, value) in attrs {
        map.insert((*key).to_owned(), Value::from(*value));
    }
    SpanView {
        host: host.to_owned(),
        service: service.to_owned(),
        name: name.to_owned(),
        span: id.to_owned(),
        parent: parent.map(str::to_owned),
        start,
        ms,
        status: if error { "error" } else { "ok" },
        message: None,
        attrs: map,
    }
}
