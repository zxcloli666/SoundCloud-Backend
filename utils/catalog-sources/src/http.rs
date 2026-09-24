use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use base64::Engine;
use bytes::Bytes;
use call_relay::{Client as RelayClient, Request as RelayRequest};
use futures::StreamExt;
use tokio::sync::Semaphore;
use wreq::header::{HeaderMap, HeaderName, HeaderValue, RETRY_AFTER};
use wreq::{Client, Method};

use crate::error::{SourceError, SourceResult};
use crate::throttle::Throttle;

const DEFAULT_JSON_LIMIT: usize = 2 * 1024 * 1024;
const RELAY_MAX_CONCURRENT: usize = 256;
const PROXY_MAX_CONCURRENT: usize = 32;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) type RelayFuture<'a, T> = Pin<Box<dyn Future<Output = SourceResult<T>> + Send + 'a>>;

#[derive(Clone)]
pub struct ExternalFetcher {
    inner: Arc<Inner>,
}

struct Inner {
    http: Client,
    proxy_url: String,
    relay: Option<Arc<dyn RelayTransport>>,
    relay_sem: Arc<Semaphore>,
    proxy_sem: Arc<Semaphore>,
}

pub(crate) struct RelayReply {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Bytes,
}

pub(crate) trait RelayTransport: Send + Sync {
    fn call_method(
        &self,
        method_id: String,
        script: String,
        inputs: Bytes,
    ) -> RelayFuture<'_, Bytes>;

    fn fetch(&self, request: RelayRequest) -> RelayFuture<'_, RelayReply>;
}

struct RealRelay {
    client: Arc<RelayClient>,
}

impl RelayTransport for RealRelay {
    fn call_method(
        &self,
        method_id: String,
        script: String,
        inputs: Bytes,
    ) -> RelayFuture<'_, Bytes> {
        Box::pin(async move {
            self.client
                .call_method(&method_id, &script, inputs)
                .await
                .map_err(|error| SourceError::Unreachable(error.to_string()))
        })
    }

    fn fetch(&self, request: RelayRequest) -> RelayFuture<'_, RelayReply> {
        Box::pin(async move {
            let response = self
                .client
                .fetch(&request)
                .await
                .map_err(|error| SourceError::Unreachable(error.to_string()))?;
            Ok(RelayReply {
                status: response.status,
                headers: response.headers,
                body: response.body,
            })
        })
    }
}

impl ExternalFetcher {
    pub fn new(http: Client, proxy_url: String, relay: Option<Arc<RelayClient>>) -> Arc<Self> {
        let relay = relay.map(|client| Arc::new(RealRelay { client }) as Arc<dyn RelayTransport>);
        Self::build(http, proxy_url, relay)
    }

    #[cfg(test)]
    pub(crate) fn new_with_transport(
        http: Client,
        proxy_url: String,
        relay: Arc<dyn RelayTransport>,
    ) -> Arc<Self> {
        Self::build(http, proxy_url, Some(relay))
    }

    fn build(http: Client, proxy_url: String, relay: Option<Arc<dyn RelayTransport>>) -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Inner {
                http,
                proxy_url,
                relay,
                relay_sem: Arc::new(Semaphore::new(RELAY_MAX_CONCURRENT)),
                proxy_sem: Arc::new(Semaphore::new(PROXY_MAX_CONCURRENT)),
            }),
        })
    }

    pub fn has_relay(&self) -> bool {
        self.inner.relay.is_some()
    }

    pub fn has_proxy(&self) -> bool {
        !self.inner.proxy_url.is_empty()
    }

    pub async fn call_method(
        &self,
        method_id: &str,
        script: &str,
        inputs: Bytes,
        max_bytes: usize,
    ) -> SourceResult<Bytes> {
        let relay = self
            .inner
            .relay
            .as_ref()
            .ok_or(SourceError::NotConfigured("relay"))?;
        let _permit = self
            .inner
            .relay_sem
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| SourceError::Unreachable("relay admission closed".to_owned()))?;
        let output = tokio::time::timeout(
            OPERATION_TIMEOUT,
            relay.call_method(method_id.to_owned(), script.to_owned(), inputs),
        )
        .await
        .map_err(|_| SourceError::Unreachable("relay method timed out".to_owned()))??;
        if output.len() > max_bytes {
            return Err(SourceError::Oversized { limit: max_bytes });
        }
        Ok(output)
    }

    pub async fn get_bytes(&self, url: &str, headers: HeaderMap) -> SourceResult<Bytes> {
        self.get_bytes_limited(url, headers, DEFAULT_JSON_LIMIT)
            .await
    }

    pub async fn get_bytes_limited(
        &self,
        url: &str,
        headers: HeaderMap,
        max_bytes: usize,
    ) -> SourceResult<Bytes> {
        let relay_error = match self
            .send_relay(
                Method::GET,
                url.to_owned(),
                headers.clone(),
                None,
                max_bytes,
            )
            .await
        {
            Ok(bytes) => return Ok(bytes),
            Err(error) => error,
        };
        match self
            .send_proxy(Method::GET, url, headers, None, max_bytes)
            .await
        {
            Ok(bytes) => Ok(bytes),
            Err(SourceError::NotConfigured(_)) => Err(relay_error),
            Err(error) => Err(error),
        }
    }

    pub async fn get_api(
        &self,
        url: &str,
        headers: HeaderMap,
        throttle: &Throttle,
    ) -> SourceResult<Bytes> {
        if headers.contains_key(wreq::header::AUTHORIZATION) {
            throttle.wait().await;
            return self
                .send_proxy(Method::GET, url, headers, None, DEFAULT_JSON_LIMIT)
                .await;
        }
        self.get_bytes(url, headers).await
    }

    pub async fn get_scrape(
        &self,
        url: &str,
        headers: HeaderMap,
        throttle: &Throttle,
    ) -> SourceResult<Bytes> {
        let relay_error = match self
            .send_relay(
                Method::GET,
                url.to_owned(),
                headers.clone(),
                None,
                4 * 1024 * 1024,
            )
            .await
        {
            Ok(bytes) => return Ok(bytes),
            Err(error) => error,
        };
        throttle.wait().await;
        match self
            .send_proxy(Method::GET, url, headers, None, 4 * 1024 * 1024)
            .await
        {
            Ok(bytes) => Ok(bytes),
            Err(SourceError::NotConfigured(_)) => Err(relay_error),
            Err(error) => Err(error),
        }
    }

    async fn send_proxy(
        &self,
        method: Method,
        target_url: &str,
        mut headers: HeaderMap,
        body: Option<Bytes>,
        max_bytes: usize,
    ) -> SourceResult<Bytes> {
        if self.inner.proxy_url.is_empty() {
            return Err(SourceError::NotConfigured("proxy"));
        }
        let _permit = self
            .inner
            .proxy_sem
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| SourceError::Unreachable("proxy admission closed".to_owned()))?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(target_url);
        headers.insert(
            HeaderName::from_static("x-target"),
            HeaderValue::from_str(&encoded)
                .map_err(|error| SourceError::Invalid(format!("target header: {error}")))?,
        );
        headers.insert(
            wreq::header::ACCEPT_ENCODING,
            HeaderValue::from_static("identity"),
        );
        let mut request = self.inner.http.request(method, &self.inner.proxy_url);
        for (name, value) in headers {
            if let Some(name) = name {
                request = request.header(name, value);
            }
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        let response = tokio::time::timeout(OPERATION_TIMEOUT, request.send())
            .await
            .map_err(|_| SourceError::Unreachable("proxy request timed out".to_owned()))?
            .map_err(|error| SourceError::Unreachable(error.to_string()))?;
        response_bytes(response, max_bytes).await
    }

    async fn send_relay(
        &self,
        method: Method,
        target_url: String,
        mut headers: HeaderMap,
        body: Option<Bytes>,
        max_bytes: usize,
    ) -> SourceResult<Bytes> {
        let relay = self
            .inner
            .relay
            .as_ref()
            .ok_or(SourceError::NotConfigured("relay"))?;
        let _permit = self
            .inner
            .relay_sem
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| SourceError::Unreachable("relay admission closed".to_owned()))?;
        headers.insert(
            wreq::header::ACCEPT_ENCODING,
            HeaderValue::from_static("identity"),
        );
        let mut relay_headers = HashMap::new();
        for (name, value) in &headers {
            if let Ok(value) = value.to_str() {
                relay_headers.insert(name.as_str().to_owned(), value.to_owned());
            }
        }
        let request = RelayRequest {
            url: target_url,
            method: method.as_str().to_owned(),
            headers: relay_headers,
            body: body.unwrap_or_default(),
        };
        let response = tokio::time::timeout(OPERATION_TIMEOUT, relay.fetch(request))
            .await
            .map_err(|_| SourceError::Unreachable("relay request timed out".to_owned()))??;
        if response.body.len() > max_bytes {
            return Err(SourceError::Oversized { limit: max_bytes });
        }
        if response.status >= 400 {
            return Err(status_error(
                response.status,
                &response.headers,
                &response.body,
            ));
        }
        Ok(response.body)
    }
}

async fn response_bytes(response: wreq::Response, max_bytes: usize) -> SourceResult<Bytes> {
    let status = response.status().as_u16();
    let retry_after_seconds = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_retry_after);
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(SourceError::Oversized { limit: max_bytes });
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| SourceError::Unreachable(error.to_string()))?;
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(SourceError::Oversized { limit: max_bytes });
        }
        bytes.extend_from_slice(&chunk);
    }
    let bytes = Bytes::from(bytes);
    if status >= 400 {
        return Err(SourceError::Status {
            status,
            body: body_preview(&bytes),
            retry_after_seconds,
        });
    }
    Ok(bytes)
}

fn status_error(status: u16, headers: &HashMap<String, String>, body: &[u8]) -> SourceError {
    let retry_after_seconds = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
        .and_then(|(_, value)| parse_retry_after(value));
    SourceError::Status {
        status,
        body: body_preview(body),
        retry_after_seconds,
    }
}

fn body_preview(body: &[u8]) -> String {
    String::from_utf8_lossy(body).chars().take(200).collect()
}

fn parse_retry_after(value: &str) -> Option<u64> {
    if let Some(seconds) = value
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|seconds| *seconds > 0)
    {
        return Some(seconds);
    }
    let retry_at = httpdate::parse_http_date(value).ok()?;
    let delay = retry_at.duration_since(SystemTime::now()).ok()?.as_secs();
    Some(delay.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_accepts_seconds_and_http_dates() {
        assert_eq!(parse_retry_after("42"), Some(42));
        let future = httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(60));
        assert!(parse_retry_after(&future).is_some_and(|seconds| (1..=60).contains(&seconds)));
    }

    #[test]
    fn retry_after_rejects_invalid_and_past_values() {
        assert_eq!(parse_retry_after("invalid"), None);
        let past = httpdate::fmt_http_date(SystemTime::now() - Duration::from_secs(60));
        assert_eq!(parse_retry_after(&past), None);
    }
}
