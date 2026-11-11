use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::future::Either;
use futures::stream::{self, StreamExt};
use redact::Secret;
use serde::Deserialize;
use serde_json::{Map, Value};
use wreq::header::{CONTENT_TYPE, HeaderName, HeaderValue};

use super::body::{Body, merge};
use super::sse::{self, Reader};
use super::{Backend, StepEvent, StepStream};
use crate::config::ProviderSettings;
use crate::observer::Failure;
use crate::request::Request;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const IDLE_CONNECTION: Duration = Duration::from_secs(90);
const MAX_REASON_LEN: usize = 48;

pub(crate) trait Wire: Send + Sync + 'static {
    type Reader: Reader + Default;

    const KEY_HEADER: HeaderName;
    const KEY_PREFIX: &'static str;
    const HEADERS: &'static [(&'static str, &'static str)];
    const SCHEMA_AT: &'static [&'static str];

    fn body(call: &Call, request: &Request) -> Value;

    fn whole(answer: &[u8]) -> Result<Vec<StepEvent>, Failure>;
}

pub(crate) struct Call {
    pub model: String,
    pub max_output_tokens: u32,
    pub stream: bool,
}

pub(crate) struct Http<W> {
    shared: Arc<Shared>,
    wire: PhantomData<fn() -> W>,
}

struct Shared {
    provider: String,
    client: wreq::Client,
    endpoint: String,
    key: Secret<String>,
    call: Call,
    extra: Map<String, Value>,
}

impl<W: Wire> Http<W> {
    pub fn new(id: &str, settings: &ProviderSettings, client: &wreq::Client) -> Option<Self> {
        Some(Self {
            shared: Arc::new(Shared {
                provider: id.to_owned(),
                client: client.clone(),
                endpoint: settings.endpoint.clone()?,
                key: settings.api_key.as_ref()?.resolve()?,
                call: Call {
                    model: settings.model.clone()?,
                    max_output_tokens: settings.max_output_tokens,
                    stream: settings.stream.unwrap_or(true),
                },
                extra: settings.body.clone().unwrap_or_default(),
            }),
            wire: PhantomData,
        })
    }
}

impl<W: Wire> Backend for Http<W> {
    fn start(&self, request: Arc<Request>) -> StepStream {
        let shared = self.shared.clone();
        async move {
            let response = match shared.open::<W>(&request).await {
                Ok(response) => response,
                Err(failure) => return Either::Right(stream::iter(vec![Err(failure)])),
            };
            if shared.call.stream {
                let provider = shared.provider.clone();
                return Either::Left(sse::events(provider, response, W::Reader::default()));
            }
            Either::Right(stream::iter(shared.whole::<W>(response).await))
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
        let key = secret_header(&format!("{}{}", W::KEY_PREFIX, self.key.expose()))
            .ok_or(Failure::Invalid)?;
        let mut value = W::body(&self.call, request);
        merge(&mut value, &self.extra);
        let body = Body {
            value,
            schema_at: W::SCHEMA_AT,
            schema: &request.schema,
        };
        let body = serde_json::to_vec(&body).map_err(|_| Failure::Invalid)?;
        let mut call = self
            .client
            .post(&self.endpoint)
            .header(CONTENT_TYPE, "application/json")
            .header(W::KEY_HEADER, key);
        for (name, value) in W::HEADERS {
            call = call.header(*name, *value);
        }
        let response = call.body(body).send().await.map_err(|error| {
            tracing::warn!(provider = %self.provider, cause = cause(&error), "llm provider is unreachable");
            Failure::Transport
        })?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let reason = response.bytes().await.ok().and_then(|body| reason(&body));
        tracing::warn!(
            provider = %self.provider,
            status = status.as_u16(),
            reason = reason.as_deref().unwrap_or("unknown"),
            "llm provider refused the request"
        );
        Err(Failure::Status)
    }

    async fn whole<W: Wire>(&self, response: wreq::Response) -> Vec<Result<StepEvent, Failure>> {
        let answer = match response.bytes().await {
            Ok(answer) => answer,
            Err(error) => {
                tracing::warn!(provider = %self.provider, cause = cause(&error), "llm provider answer broke off");
                return vec![Err(Failure::Transport)];
            }
        };
        match W::whole(&answer) {
            Ok(events) => events.into_iter().map(Ok).collect(),
            Err(failure) => {
                tracing::warn!(provider = %self.provider, ?failure, "llm provider answer is unusable");
                vec![Err(failure)]
            }
        }
    }
}

fn secret_header(value: &str) -> Option<HeaderValue> {
    let mut header = HeaderValue::from_str(value).ok()?;
    header.set_sensitive(true);
    Some(header)
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
