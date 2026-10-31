use std::borrow::Cow;
use std::time::Duration;

use opentelemetry::trace::Status;
use opentelemetry::{Array, Context, KeyValue, StringValue, Value};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{Span, SpanData, SpanProcessor};

pub const MASK: &str = "[redacted]";

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

const PLAIN_KEYS: [&str; 1] = ["request_id"];

const SECRET_WORDS: [&str; 10] = [
    "token",
    "secret",
    "passw",
    "key",
    "auth",
    "session",
    "client_id",
    "credential",
    "cookie",
    "signature",
];

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
    match text(input) {
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
    DROPPED_KEYS.contains(&key) || secret_key(key)
}

fn secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_WORDS.iter().any(|word| lower.contains(word))
}

pub fn text(input: &str) -> Cow<'_, str> {
    if input.len() < 16 && !input.contains([':', '=', '/', '?', ' ']) {
        return Cow::Borrowed(input);
    }
    let without_uri = strip_uri_note(input);
    let mut out = String::with_capacity(without_uri.len());
    let mut hide_next = false;
    let mut rest = without_uri.as_str();
    while !rest.is_empty() {
        let space = rest.find(char::is_whitespace);
        let (word, tail) = match space {
            Some(at) => rest.split_at(at),
            None => (rest, ""),
        };
        out.push_str(&word_text(word, &mut hide_next));
        let gap = tail.len() - tail.trim_start().len();
        out.push_str(&tail[..gap]);
        rest = &tail[gap..];
    }
    if out == input {
        Cow::Borrowed(input)
    } else {
        Cow::Owned(out)
    }
}

fn strip_uri_note(input: &str) -> String {
    let Some(start) = input.find(" for uri (") else {
        return input.to_owned();
    };
    let after = &input[start + " for uri (".len()..];
    match after.find(')') {
        Some(end) => format!("{}{}", &input[..start], &after[end + 1..]),
        None => input[..start].to_owned(),
    }
}

fn word_text(word: &str, hide_next: &mut bool) -> String {
    let lead = word.len() - word.trim_start_matches(is_wrapper).len();
    let core_end = word.trim_end_matches(is_wrapper).len().max(lead);
    let (head, core, tail) = (&word[..lead], &word[lead..core_end], &word[core_end..]);
    format!("{head}{}{tail}", core_text(core, hide_next))
}

fn is_wrapper(character: char) -> bool {
    matches!(
        character,
        '(' | ')' | '"' | '\'' | '<' | '>' | '[' | ']' | '{' | '}' | ',' | ';' | '.'
    )
}

fn core_text(core: &str, hide_next: &mut bool) -> String {
    if core.is_empty() {
        return String::new();
    }
    let lower = core.to_ascii_lowercase();
    if matches!(lower.as_str(), "bearer" | "oauth" | "basic") {
        *hide_next = true;
        return core.to_owned();
    }
    if *hide_next {
        *hide_next = false;
        return MASK.to_owned();
    }
    if let Some(at) = core.find("://") {
        return url_text(core, at);
    }
    if let Some(at) = core.find(['=', ':']) {
        let (key, value) = core.split_at(at);
        let separator = &value[..1];
        let value = &value[1..];
        let key_name = key.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
        if secret_key(key_name) {
            if value.is_empty() {
                *hide_next = true;
                return core.to_owned();
            }
            return format!("{key}{separator}{MASK}");
        }
        if value.starts_with(':') {
            return core.to_owned();
        }
        return format!("{key}{separator}{}", core_text(value, hide_next));
    }
    if secret_shaped(core) {
        MASK.to_owned()
    } else {
        core.to_owned()
    }
}

fn url_text(core: &str, scheme_end: usize) -> String {
    let scheme = &core[..scheme_end];
    let rest = &core[scheme_end + 3..];
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    let location = &rest[..end];
    let (authority, path) = match location.find('/') {
        Some(at) => location.split_at(at),
        None => (location, ""),
    };
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let path = path
        .split('/')
        .map(|segment| {
            if secret_shaped(segment) || share_token(segment) {
                MASK
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    format!("{scheme}://{host}{path}")
}

fn share_token(segment: &str) -> bool {
    segment.len() > 3
        && segment.starts_with("s-")
        && segment[2..].bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn secret_shaped(word: &str) -> bool {
    if word.starts_with("eyJ") && word.contains('.') && word.len() > 20 {
        return true;
    }
    if word.len() < 24 || is_uuid(word) {
        return false;
    }
    let plain = word
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    plain
        && word.bytes().any(|byte| byte.is_ascii_digit())
        && word.bytes().any(|byte| byte.is_ascii_alphabetic())
}

fn is_uuid(word: &str) -> bool {
    word.len() == 36
        && word.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
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
    fn a_wreq_error_loses_its_uri_and_query() {
        let message = format!(
            "error sending request for uri (https://api-v2.soundcloud.com/tracks/1?client_id={CLIENT_ID}&limit=5): connection reset"
        );

        let cleaned = text(&message);

        assert!(!forbidden(&cleaned), "{cleaned}");
        assert!(cleaned.contains("connection reset"));
    }

    #[test]
    fn urls_keep_host_and_path_but_never_the_query_or_userinfo() {
        let cleaned = text(&format!(
            "GET https://user:pass@api.soundcloud.com/playlists/9?secret_token=s-AbC12&client_id={CLIENT_ID} failed"
        ));

        assert_eq!(cleaned, "GET https://api.soundcloud.com/playlists/9 failed");
    }

    #[test]
    fn secret_shaped_values_are_masked_wherever_they_sit() {
        for message in [
            format!("rejected {CLIENT_ID} twice"),
            format!("Authorization: OAuth {OAUTH}"),
            format!("Bearer {OAUTH}"),
            format!("client_id={CLIENT_ID}"),
            format!("{{\"client_id\":\"{CLIENT_ID}\"}}"),
            format!("oauth_token: {OAUTH}"),
            "https://soundcloud.com/artist/track/s-AbCdE12".to_owned(),
        ] {
            let cleaned = text(&message);
            assert!(!forbidden(&cleaned), "{cleaned}");
            assert!(!cleaned.contains("s-AbCdE12"), "{cleaned}");
        }
    }

    #[test]
    fn ordinary_messages_stay_as_they_are() {
        for message in [
            "database error: pool timed out",
            "request failed",
            "550e8400-e29b-41d4-a716-446655440000 not found",
            "modules::auth::service refused",
            "GET /tracks/{id}",
        ] {
            assert!(matches!(text(message), Cow::Borrowed(_)), "{message}");
        }
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
