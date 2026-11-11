use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::stream::{self, StreamExt};
use serde::Deserialize;
use serde_json::{Map, Value};
use wreq::header::{CONTENT_TYPE, HeaderName, HeaderValue};

use super::body::{Body, merge};
use super::fault;
use super::sse::{self, Reader};
use super::{Backend, StepEvent, StepStream};
use crate::config::ProviderSettings;
use crate::observer::Failure;
use crate::request::Request;

pub(crate) const MAX_ANSWER_BYTES: usize = 4 * 1024 * 1024;
const MAX_ERROR_BYTES: usize = 64 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const IDLE_CONNECTION: Duration = Duration::from_secs(90);
const MAX_REASON_LEN: usize = 48;

pub(crate) trait Wire: Send + Sync + 'static {
    type Reader: Reader + Default;

    const KEY_HEADER: HeaderName;
    const KEY_PREFIX: &'static str;
    const SCHEMA_AT: &'static [&'static str];

    fn body(call: &Call, request: &Request) -> Value;

    fn whole(answer: &[u8]) -> Vec<Result<StepEvent, Failure>>;
}

pub(crate) struct Call {
    pub model: String,
    pub max_output_tokens: u32,
    pub stream: bool,
    pub prefix_cache: Option<Map<String, Value>>,
}

pub(crate) struct Http<W> {
    shared: Arc<Shared>,
    wire: PhantomData<fn() -> W>,
}

struct Shared {
    provider: String,
    client: wreq::Client,
    endpoint: String,
    headers: Vec<(HeaderName, HeaderValue)>,
    call: Call,
    extra: Map<String, Value>,
}

enum Unread {
    Broke(&'static str),
    TooLarge,
}

impl<W: Wire> Http<W> {
    pub fn new(id: &str, settings: &ProviderSettings, client: &wreq::Client) -> Option<Self> {
        let key = settings.api_key.as_ref()?.resolve()?;
        let mut key = HeaderValue::from_str(&format!("{}{}", W::KEY_PREFIX, key.expose())).ok()?;
        key.set_sensitive(true);
        let mut headers = vec![
            (CONTENT_TYPE, HeaderValue::from_static("application/json")),
            (W::KEY_HEADER, key),
        ];
        for (name, value) in settings.headers.iter().flatten() {
            let name = HeaderName::from_bytes(name.as_bytes()).ok()?;
            headers.push((name, HeaderValue::from_str(value).ok()?));
        }
        Some(Self {
            shared: Arc::new(Shared {
                provider: id.to_owned(),
                client: client.clone(),
                endpoint: settings.endpoint.clone()?,
                headers,
                call: Call {
                    model: settings.model.clone()?,
                    max_output_tokens: settings.max_output_tokens,
                    stream: settings.stream.unwrap_or(true),
                    prefix_cache: settings.prefix_cache.clone(),
                },
                extra: settings.body.clone().unwrap_or_default(),
            }),
            wire: PhantomData,
        })
    }
}

impl<W: Wire> Backend for Http<W> {
    fn start(&self, request: Arc<Request>, _time_left: Duration) -> StepStream {
        let shared = self.shared.clone();
        async move {
            match shared.open::<W>(&request).await {
                Ok(response) => stream::iter([Ok(StepEvent::Accepted)])
                    .chain(shared.answer::<W>(response))
                    .boxed(),
                Err(failure) => stream::iter([Err(failure)]).boxed(),
            }
        }
        .flatten_stream()
        .boxed()
    }
}

pub(crate) fn client() -> Result<wreq::Client, wreq::Error> {
    wreq::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_idle_timeout(IDLE_CONNECTION)
        .build()
}

pub(crate) fn cause(error: &wreq::Error) -> &'static str {
    if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_body() || error.is_decode() {
        "body"
    } else {
        "other"
    }
}

impl Shared {
    async fn open<W: Wire>(&self, request: &Request) -> Result<wreq::Response, Failure> {
        let mut value = W::body(&self.call, request);
        merge(&mut value, &self.extra);
        let body = Body {
            value,
            schema_at: W::SCHEMA_AT,
            schema: &request.schema,
        };
        let body = serde_json::to_vec(&body).map_err(|_| Failure::Request)?;
        let mut call = self.client.post(&self.endpoint);
        for (name, value) in &self.headers {
            call = call.header(name, value);
        }
        let response = call.body(body).send().await.map_err(|error| {
            tracing::warn!(provider = %self.provider, cause = cause(&error), "llm provider is unreachable");
            Failure::Transport
        })?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let reason = read_capped(response, MAX_ERROR_BYTES)
            .await
            .ok()
            .and_then(|body| reason(&body));
        tracing::warn!(
            provider = %self.provider,
            status = status.as_u16(),
            reason = reason.as_deref().unwrap_or("unknown"),
            "llm provider refused the request"
        );
        Err(fault::of_status(status.as_u16()))
    }

    fn answer<W: Wire>(self: Arc<Self>, response: wreq::Response) -> StepStream {
        if self.call.stream {
            let provider = self.provider.clone();
            return sse::events(provider, response, W::Reader::default()).boxed();
        }
        stream::once(async move { self.whole::<W>(response).await })
            .flat_map(stream::iter)
            .boxed()
    }

    async fn whole<W: Wire>(&self, response: wreq::Response) -> Vec<Result<StepEvent, Failure>> {
        let answer = match read_capped(response, MAX_ANSWER_BYTES).await {
            Ok(answer) => answer,
            Err(Unread::Broke(cause)) => {
                tracing::warn!(provider = %self.provider, cause, "llm provider answer broke off");
                return vec![Err(Failure::Transport)];
            }
            Err(Unread::TooLarge) => {
                tracing::warn!(provider = %self.provider, limit = MAX_ANSWER_BYTES, "llm provider answer is too large");
                return vec![Err(Failure::Transport)];
            }
        };
        let events = W::whole(&answer);
        if let Some(Err(failure)) = events.last() {
            tracing::warn!(provider = %self.provider, ?failure, "llm provider answer is unusable");
        }
        events
    }
}

async fn read_capped(response: wreq::Response, cap: usize) -> Result<Vec<u8>, Unread> {
    let mut body = Vec::new();
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|error| Unread::Broke(cause(&error)))?;
        if body.len() + chunk.len() > cap {
            return Err(Unread::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn reason(body: &[u8]) -> Option<String> {
    #[derive(Deserialize)]
    struct Envelope {
        error: Option<Problem>,
    }
    #[derive(Deserialize)]
    struct Problem {
        #[serde(rename = "type")]
        kind: Option<Value>,
        code: Option<Value>,
    }
    let problem = serde_json::from_slice::<Envelope>(body).ok()?.error?;
    let label = [problem.kind, problem.code]
        .into_iter()
        .flatten()
        .find_map(|value| match value {
            Value::String(text) => Some(text),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })?;
    Some(
        label
            .chars()
            .filter(|symbol| symbol.is_ascii_alphanumeric() || matches!(symbol, '_' | '-' | '.'))
            .take(MAX_REASON_LEN)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_short_label_is_taken_from_an_error_body() {
        let typed =
            br#"{"type":"error","error":{"type":"overloaded_error","message":"secret words"}}"#;
        assert_eq!(reason(typed).as_deref(), Some("overloaded_error"));
        let coded = br#"{"error":{"message":"secret words","type":null,"code":"rate limit!"}}"#;
        assert_eq!(reason(coded).as_deref(), Some("ratelimit"));
        assert_eq!(reason(b"<html>bad gateway</html>"), None);
    }
}
