use std::net::{IpAddr, SocketAddr};

use backend_contracts::diag_report::{Report, TimelineEntry, View, Who};

const INTERNAL_DOMAINS: [&str; 3] = ["scnative.", "scdinternal.", ".internal"];
const INTERNAL_SERVICES: [&str; 12] = [
    "backend",
    "streaming",
    "storage",
    "haproxy",
    "jaeger",
    "postgres",
    "redis",
    "nats",
    "qdrant",
    "minio",
    "call",
    "worker",
];

pub fn redact(report: &mut Report) {
    report.header.view = View::Public;
    report.header.app.id = None;
    for value in report.header.session.route.values_mut() {
        *value = scrub(value);
    }
    for entry in &mut report.timeline {
        redact_entry(entry);
    }
    for error in &mut report.errors {
        error.trace = None;
        error.message = scrub(&error.message);
        error.component_stack = error.component_stack.as_deref().map(scrub);
        for value in error.context.values_mut() {
            *value = scrub(value);
        }
        for frame in &mut error.frames {
            frame.function = frame.function.as_deref().map(scrub);
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
        Who::Server => format!("{} {}", entry.host, entry.service),
    };
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
    match hop.chars().next() {
        Some('r') => format!("relay {hop}"),
        Some('p') => format!("pro {hop}"),
        _ => hop.to_owned(),
    }
}

fn source_role(source: &str) -> &'static str {
    match source {
        "relay_track" => "relay",
        "oauth" => "user session",
        "anon" => "public source",
        _ => "backup source",
    }
}

fn upstream_role(route: Option<&str>) -> &'static str {
    match route {
        Some("relay") => "relay",
        Some("direct") => "public source",
        _ => "backup source",
    }
}

fn scrub(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    for character in text.chars() {
        if character.is_alphanumeric() || ".:-_@%[]".contains(character) {
            token.push(character);
        } else {
            out.push_str(&scrub_token(&token));
            token.clear();
            out.push(character);
        }
    }
    out.push_str(&scrub_token(&token));
    out
}

fn scrub_token(token: &str) -> String {
    let core = token.trim_end_matches(['.', ':', ',']);
    let tail = &token[core.len()..];
    let replaced = replacement(core);
    match replaced {
        Some(replaced) => format!("{replaced}{tail}"),
        None => token.to_owned(),
    }
}

fn replacement(core: &str) -> Option<String> {
    if core.is_empty() {
        return None;
    }
    if core.contains('@') {
        return Some("[email]".to_owned());
    }
    let bare = core.replace(['[', ']'], "");
    if is_address(&bare) {
        return Some("[ip]".to_owned());
    }
    let lower = bare.to_ascii_lowercase();
    let is_hex =
        |length: usize| lower.len() == length && lower.bytes().all(|b| b.is_ascii_hexdigit());
    if is_hex(32) || is_hex(12) {
        return Some("[id]".to_owned());
    }
    if INTERNAL_DOMAINS.iter().any(|domain| lower.contains(domain)) {
        return Some("[host]".to_owned());
    }
    if let Some((name, port)) = lower.split_once(':')
        && INTERNAL_SERVICES.contains(&name)
        && !port.is_empty()
        && port.bytes().all(|b| b.is_ascii_digit())
    {
        return Some("[host]".to_owned());
    }
    match lower.as_str() {
        "relay_track" => Some("relay".to_owned()),
        "oauth" => Some("user session".to_owned()),
        "anon" => Some("public source".to_owned()),
        "cookies" | "restricted" | "cdn_proxy" | "cdn_redirect" => Some("backup source".to_owned()),
        _ => None,
    }
}

fn is_address(text: &str) -> bool {
    if text.parse::<IpAddr>().is_ok() || text.parse::<SocketAddr>().is_ok() {
        return true;
    }
    match text.rsplit_once(':') {
        Some((host, port)) => {
            host.parse::<IpAddr>().is_ok()
                && !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    }
}
