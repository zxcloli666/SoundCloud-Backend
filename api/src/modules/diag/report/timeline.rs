use std::collections::{BTreeMap, HashMap};

use backend_contracts::diag::{Attempt, Bundle};
use backend_contracts::diag_report::{Outcome, TimelineEntry, Who};
use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::Value;

use super::super::jaeger::{SpanView, TraceView};

pub struct Timeline {
    pub entries: Vec<TimelineEntry>,
    pub attempts: Vec<usize>,
    pub spans: HashMap<(String, String), usize>,
    pub offset_ms: Option<i64>,
}

enum Origin {
    Attempt(usize),
    Span(String, String),
}

pub fn build(bundle: &Bundle, traces: &[TraceView]) -> Timeline {
    let offset_ms = clock_offset(&bundle.attempts, traces);
    let shift = offset_ms.unwrap_or(0);
    let fallback = millis(&bundle.at).unwrap_or(0);
    let mut items: Vec<(i64, Origin, TimelineEntry)> = Vec::new();
    for (index, attempt) in bundle.attempts.iter().enumerate() {
        let at = millis(&attempt.at).unwrap_or(fallback);
        items.push((at, Origin::Attempt(index), client_entry(attempt, at)));
    }
    for trace in traces {
        for span in &trace.spans {
            let at = millis(&span.start).unwrap_or(fallback) - shift;
            let origin = Origin::Span(trace.trace.clone(), span.span.clone());
            items.push((at, origin, server_entry(&trace.trace, span, at)));
        }
    }
    items.sort_by_key(|(at, origin, _)| (*at, matches!(origin, Origin::Span(..))));

    let mut timeline = Timeline {
        entries: Vec::new(),
        attempts: vec![0; bundle.attempts.len()],
        spans: HashMap::new(),
        offset_ms,
    };
    for (_, origin, entry) in items {
        let repeated = matches!(timeline.entries.last(), Some(last) if same_line(last, &entry));
        if repeated && let Some(last) = timeline.entries.last_mut() {
            last.count += 1;
        } else {
            timeline.entries.push(entry);
        }
        let index = timeline.entries.len() - 1;
        match origin {
            Origin::Attempt(attempt) => timeline.attempts[attempt] = index,
            Origin::Span(trace, span) => {
                timeline.spans.insert((trace, span), index);
            }
        }
    }
    timeline
}

fn clock_offset(attempts: &[Attempt], traces: &[TraceView]) -> Option<i64> {
    let mut offsets: Vec<i64> = Vec::new();
    for attempt in attempts {
        let Some(trace) = traces.iter().find(|trace| trace.trace == attempt.trace) else {
            continue;
        };
        let root = trace
            .spans
            .iter()
            .find(|span| span.parent.as_deref() == Some(attempt.span.as_str()));
        if let (Some(root), Some(sent)) = (root, millis(&attempt.at))
            && let Some(received) = millis(&root.start)
        {
            offsets.push(received - sent);
        }
    }
    offsets.sort_unstable();
    let middle = offsets.len() / 2;
    match offsets.len() {
        0 => None,
        length if length % 2 == 1 => Some(offsets[middle]),
        _ => Some((offsets[middle - 1] + offsets[middle]) / 2),
    }
}

fn client_entry(attempt: &Attempt, at: i64) -> TimelineEntry {
    let failed = attempt.status == 0 || attempt.status >= 400 || attempt.error.is_some();
    let mut attrs = BTreeMap::new();
    if attempt.bytes > 0 {
        attrs.insert("bytes".to_owned(), attempt.bytes.to_string());
    }
    TimelineEntry {
        at: iso(at),
        who: Who::Client,
        host: attempt.hop.clone(),
        service: attempt.service.as_str().to_owned(),
        name: format!("{} {}", attempt.method, attempt.path),
        outcome: if failed { Outcome::Error } else { Outcome::Ok },
        status: Some(attempt.status).filter(|status| *status > 0),
        ms: attempt.ms,
        message: attempt.error.clone(),
        count: 1,
        attrs,
        trace: Some(attempt.trace.clone()),
    }
}

fn server_entry(trace: &str, span: &SpanView, at: i64) -> TimelineEntry {
    let service = span
        .service
        .strip_suffix(&format!("-{}", span.host))
        .unwrap_or(&span.service);
    let status = span
        .attrs
        .get("http.response.status_code")
        .and_then(Value::as_u64)
        .and_then(|status| u16::try_from(status).ok());
    let attrs = span
        .attrs
        .iter()
        .filter(|(key, _)| {
            key.as_str() != "scd.host" && key.as_str() != "http.response.status_code"
        })
        .map(|(key, value)| (key.clone(), plain(value)))
        .collect();
    TimelineEntry {
        at: iso(at),
        who: Who::Server,
        host: span.host.clone(),
        service: service.to_owned(),
        name: span.name.clone(),
        outcome: if span.status == "error" {
            Outcome::Error
        } else {
            Outcome::Ok
        },
        status,
        ms: span.ms,
        message: span.message.clone(),
        count: 1,
        attrs,
        trace: Some(trace.to_owned()),
    }
}

fn same_line(a: &TimelineEntry, b: &TimelineEntry) -> bool {
    a.who == b.who
        && a.host == b.host
        && a.service == b.service
        && a.name == b.name
        && a.outcome == b.outcome
        && a.status == b.status
        && a.message == b.message
}

fn plain(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

pub fn millis(text: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.timestamp_millis())
}

fn iso(millis: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(millis)
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}
