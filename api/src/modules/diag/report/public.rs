use backend_contracts::diag_report::{Report, TimelineEntry, View, Who};
use chrono::{DateTime, SecondsFormat, Utc};

use super::scrub::{scrub, source_role};
use super::timeline::millis;

pub fn redact(report: &mut Report) {
    let header = &mut report.header;
    header.view = View::Public;
    header.created = millis(&header.created)
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .map_or_else(String::new, |at| {
            at.to_rfc3339_opts(SecondsFormat::Millis, true)
        });
    header.code = scrub(&header.code);
    header.app.id = None;
    for field in [
        &mut header.app.version,
        &mut header.app.os,
        &mut header.app.os_version,
        &mut header.app.arch,
        &mut header.app.install,
        &mut header.app.locale,
        &mut header.session.quality,
    ] {
        *field = scrub(field);
    }
    header.session.route = std::mem::take(&mut header.session.route)
        .into_iter()
        .map(|(key, value)| (scrub(&key), scrub(&value)))
        .collect();
    for entry in &mut report.timeline {
        redact_entry(entry);
    }
    for error in &mut report.errors {
        error.trace = None;
        error.message = scrub(&error.message);
        error.component_stack = error.component_stack.as_deref().map(scrub);
        error.context = std::mem::take(&mut error.context)
            .into_iter()
            .map(|(key, value)| (scrub(&key), scrub(&value)))
            .collect();
        for frame in &mut error.frames {
            frame.file = scrub(&frame.file);
            frame.function = frame.function.as_deref().map(scrub);
            frame.source = frame.source.as_deref().map(scrub);
            frame.symbol = frame.symbol.as_deref().map(scrub);
        }
    }
    for hint in &mut report.hints {
        hint.text = scrub(&hint.text);
    }
    report.missing.trace_ids.clear();
}

fn redact_entry(entry: &mut TimelineEntry) {
    entry.trace = None;
    entry.host = match entry.who {
        Who::Client => client_role(&entry.host),
        Who::Server => scrub(&format!("{} {}", entry.host, entry.service)),
    };
    entry.service = scrub(&entry.service);
    entry.attrs.remove("scd.host");
    let route = entry.attrs.remove("scd.route");
    if let Some(source) = entry.attrs.get_mut("scd.source") {
        *source = source_role(source).to_owned();
    }
    if let Some(upstream) = entry.attrs.get_mut("scd.upstream") {
        *upstream = upstream_role(route.as_deref()).to_owned();
    }
    for value in entry.attrs.values_mut() {
        *value = scrub(value);
    }
    entry.name = match (
        entry.attrs.get("scd.source"),
        entry.attrs.get("scd.upstream"),
    ) {
        (Some(role), _) if entry.name.starts_with("source ") => format!("source {role}"),
        (_, Some(role)) if entry.name.starts_with("upstream ") => format!("upstream {role}"),
        _ => scrub(&entry.name),
    };
    entry.message = entry.message.as_deref().map(scrub);
}

fn client_role(hop: &str) -> String {
    let numbered = |prefix: char| {
        hop.strip_prefix(prefix)
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
    };
    if numbered('r') {
        format!("relay {hop}")
    } else if numbered('p') {
        format!("pro {hop}")
    } else {
        scrub(hop)
    }
}

fn upstream_role(route: Option<&str>) -> &'static str {
    match route {
        Some("relay") => "relay",
        Some("direct") => "public source",
        _ => "backup source",
    }
}
