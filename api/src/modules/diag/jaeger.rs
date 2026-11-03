use chrono::{DateTime, SecondsFormat, Utc};
use futures::StreamExt;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::error::AppError;

const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_SPANS: usize = 2000;

const KEPT_ATTRIBUTES: [&str; 13] = [
    "http.request.method",
    "http.route",
    "http.response.status_code",
    "scd.debug",
    "scd.host",
    "scd.source",
    "scd.tier",
    "scd.outcome",
    "scd.upstream",
    "scd.route",
    "scd.status",
    "bytes",
    "error.message",
];

#[derive(Debug, Serialize)]
pub struct SpanView {
    pub host: String,
    pub service: String,
    pub name: String,
    pub span: String,
    pub parent: Option<String>,
    pub start: String,
    pub ms: u64,
    pub status: &'static str,
    pub message: Option<String>,
    pub attrs: Map<String, Value>,
}

#[derive(Debug, Serialize)]
pub struct TraceView {
    pub trace: String,
    pub spans: Vec<SpanView>,
}

pub async fn fetch(
    http: &wreq::Client,
    base: &str,
    trace: &str,
) -> Result<Option<TraceView>, AppError> {
    let url = format!("{}/api/v3/traces/{trace}", base.trim_end_matches('/'));
    let response = http
        .get(url)
        .send()
        .await
        .map_err(|error| AppError::service_unavailable(format!("jaeger: {error}")))?;
    if response.status().as_u16() == 404 {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(AppError::service_unavailable(format!(
            "jaeger answered {}",
            response.status()
        )));
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|error| AppError::service_unavailable(format!("jaeger: {error}")))?;
        body.extend_from_slice(&chunk);
        if body.len() > MAX_BODY_BYTES {
            return Err(AppError::service_unavailable("jaeger answer is too large"));
        }
    }
    Ok(compact(trace, &body))
}

pub fn compact(trace: &str, body: &[u8]) -> Option<TraceView> {
    let mut spans = Vec::new();
    for chunk in serde_json::Deserializer::from_slice(body).into_iter::<Value>() {
        let Ok(chunk) = chunk else {
            break;
        };
        let resources = chunk
            .pointer("/result/resourceSpans")
            .or_else(|| chunk.get("resourceSpans"))
            .and_then(Value::as_array);
        for resource in resources.into_iter().flatten() {
            let attributes = resource.pointer("/resource/attributes");
            let service = attribute(attributes, "service.name").unwrap_or_default();
            let host = attribute(attributes, "scd.host").unwrap_or_default();
            let scopes = resource.get("scopeSpans").and_then(Value::as_array);
            for scope in scopes.into_iter().flatten() {
                let list = scope.get("spans").and_then(Value::as_array);
                for span in list.into_iter().flatten().take(MAX_SPANS - spans.len()) {
                    spans.push(view(span, &service, &host));
                }
            }
        }
    }
    if spans.is_empty() {
        return None;
    }
    spans.sort_by(|a, b| a.start.cmp(&b.start));
    Some(TraceView {
        trace: trace.to_owned(),
        spans,
    })
}

fn view(span: &Value, service: &str, host: &str) -> SpanView {
    let start_nanos = nanos(span.get("startTimeUnixNano"));
    let end_nanos = nanos(span.get("endTimeUnixNano"));
    let start = DateTime::<Utc>::from_timestamp_nanos(start_nanos as i64)
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let errored = match span.pointer("/status/code") {
        Some(Value::Number(code)) => code.as_i64() == Some(2),
        Some(Value::String(code)) => code.contains("ERROR"),
        _ => false,
    };
    let message = span
        .pointer("/status/message")
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
        .map(str::to_owned);
    let parent = span
        .get("parentSpanId")
        .and_then(Value::as_str)
        .filter(|parent| !parent.is_empty())
        .map(str::to_owned);
    let mut attrs = Map::new();
    let list = span.get("attributes").and_then(Value::as_array);
    for item in list.into_iter().flatten() {
        let Some(key) = item.get("key").and_then(Value::as_str) else {
            continue;
        };
        let key = match key {
            "http.status_code" => "http.response.status_code",
            other => other,
        };
        if KEPT_ATTRIBUTES.contains(&key)
            && let Some(value) = item.get("value").and_then(plain)
        {
            attrs.insert(key.to_owned(), value);
        }
    }
    SpanView {
        host: host.to_owned(),
        service: service.to_owned(),
        name: span
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        span: span
            .get("spanId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        parent,
        start,
        ms: end_nanos.saturating_sub(start_nanos) / 1_000_000,
        status: if errored { "error" } else { "ok" },
        message,
        attrs,
    }
}

fn nanos(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::String(text)) => text.parse().unwrap_or(0),
        Some(Value::Number(number)) => number.as_u64().unwrap_or(0),
        _ => 0,
    }
}

fn attribute(list: Option<&Value>, key: &str) -> Option<String> {
    list?
        .as_array()?
        .iter()
        .find(|item| item.get("key").and_then(Value::as_str) == Some(key))?
        .get("value")
        .and_then(plain)
        .and_then(|value| value.as_str().map(str::to_owned))
}

fn plain(value: &Value) -> Option<Value> {
    if let Some(text) = value.get("stringValue") {
        return Some(text.clone());
    }
    if let Some(flag) = value.get("boolValue") {
        return Some(flag.clone());
    }
    if let Some(number) = value.get("intValue") {
        let parsed = match number {
            Value::String(text) => text.parse::<i64>().ok()?,
            other => other.as_i64()?,
        };
        return Some(Value::from(parsed));
    }
    value.get("doubleValue").cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = r#"{"result":{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"api-main"}},{"key":"scd.host","value":{"stringValue":"main"}}]},"scopeSpans":[{"spans":[
        {"traceId":"4bf92f3577b3da6a3ce929d0e0e4736f","spanId":"aaaaaaaaaaaaaaaa","parentSpanId":"bbbbbbbbbbbbbbbb","name":"GET /health","startTimeUnixNano":"1760000000000000000","endTimeUnixNano":"1760000000250000000","status":{"code":2,"message":"HTTP 502"},"attributes":[{"key":"http.response.status_code","value":{"intValue":"502"}},{"key":"request_id","value":{"stringValue":"secret-ish"}},{"key":"scd.debug","value":{"boolValue":true}}]}
    ]}]}]}}"#;

    #[test]
    fn a_v3_trace_becomes_a_compact_span_list() {
        let view = compact("4bf92f3577b3da6a3ce929d0e0e4736f", BODY.as_bytes()).expect("spans");
        let span = &view.spans[0];

        assert_eq!(span.service, "api-main");
        assert_eq!(span.host, "main");
        assert_eq!(span.ms, 250);
        assert_eq!(span.status, "error");
        assert_eq!(span.message.as_deref(), Some("HTTP 502"));
        assert_eq!(span.parent.as_deref(), Some("bbbbbbbbbbbbbbbb"));
        assert_eq!(span.attrs["http.response.status_code"], 502);
        assert_eq!(span.attrs["scd.debug"], true);
        assert!(!span.attrs.contains_key("request_id"));
    }

    const STORED: &str = r#"{"result":{"resourceSpans":[{"resource":{"attributes":[{"key":"scd.host","value":{"stringValue":"main"}},{"key":"service.name","value":{"stringValue":"api-main"}},{"key":"telemetry.sdk.language","value":{"stringValue":"rust"}},{"key":"telemetry.sdk.name","value":{"stringValue":"opentelemetry"}},{"key":"telemetry.sdk.version","value":{"stringValue":"0.33.1"}}]},"scopeSpans":[{"scope":{"name":"api"},"spans":[{"traceId":"e5de4e36a0aeef2184790ed7327e2ac7","spanId":"d536487b0a014319","flags":1,"name":"GET /health","kind":2,"startTimeUnixNano":"1793300100351847885","endTimeUnixNano":"1793300100352286235","attributes":[{"key":"http.request.method","value":{"stringValue":"GET"}},{"key":"http.route","value":{"stringValue":"/health"}},{"key":"http.status_code","value":{"intValue":"503"}},{"key":"request_id","value":{"stringValue":"e5de4e36a0aeef2184790ed7327e2ac7"}},{"key":"scd.host","value":{"stringValue":"main"}}],"status":{"code":2}}]}]}]}}"#;

    #[test]
    fn the_status_code_jaeger_stores_under_its_old_name_is_read() {
        let view = compact("e5de4e36a0aeef2184790ed7327e2ac7", STORED.as_bytes()).expect("spans");
        let span = &view.spans[0];

        assert_eq!(span.attrs["http.response.status_code"], 503);
        assert!(!span.attrs.contains_key("http.status_code"));
        assert_eq!(span.status, "error");
    }

    #[test]
    fn nothing_in_the_body_means_no_trace() {
        assert!(compact("x", br#"{"result":{"resourceSpans":[]}}"#).is_none());
        assert!(compact("x", b"").is_none());
    }

    #[test]
    fn a_trace_keeps_at_most_two_thousand_spans() {
        let span = r#"{"traceId":"4bf92f3577b3da6a3ce929d0e0e4736f","spanId":"aaaaaaaaaaaaaaaa","name":"GET /x","startTimeUnixNano":"1760000000000000000","endTimeUnixNano":"1760000000001000000"}"#;
        let spans = vec![span; 20_000].join(",");
        let body = format!(
            r#"{{"result":{{"resourceSpans":[{{"resource":{{"attributes":[]}},"scopeSpans":[{{"spans":[{spans}]}}]}}]}}}}"#
        );

        let view = compact("4bf92f3577b3da6a3ce929d0e0e4736f", body.as_bytes()).expect("spans");

        assert_eq!(view.spans.len(), MAX_SPANS);
    }

    #[tokio::test]
    async fn an_answer_over_eight_megabytes_is_refused() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let size = MAX_BODY_BYTES + 1024;
            let head =
                format!("HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n");
            stream.write_all(head.as_bytes()).await.ok();
            stream.write_all(&vec![b' '; size]).await.ok();
        });
        let http = wreq::Client::builder().build().expect("client");

        let result = fetch(
            &http,
            &format!("http://127.0.0.1:{port}"),
            "4bf92f3577b3da6a3ce929d0e0e4736f",
        )
        .await;

        assert!(result.is_err());
    }
}
