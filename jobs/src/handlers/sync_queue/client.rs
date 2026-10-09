use std::time::Duration;

use base64::Engine;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use url::Url;
use wreq::header::{
    ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, CONTENT_TYPE, HeaderValue, RETRY_AFTER,
};
use wreq::{Client, Method, StatusCode};

use crate::config::{OAuthConfig, SyncQueueConfig};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub struct SoundCloudClient {
    http: Client,
    api_url: Url,
    proxy_url: Option<Url>,
}

pub struct TokenRefreshClient {
    http: Client,
    token_url: Url,
}

#[derive(Debug, thiserror::Error)]
pub enum SoundCloudError {
    #[error("SoundCloud request failed: {0}")]
    Transport(#[from] wreq::Error),

    #[error("SoundCloud returned HTTP {status}")]
    Api {
        status: StatusCode,
        body: Value,
        retry_after_seconds: Option<i64>,
    },

    #[error("SoundCloud response exceeded the size limit")]
    ResponseTooLarge,

    #[error("SoundCloud returned an invalid token response")]
    InvalidTokenResponse,
}

pub struct RefreshedToken {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: DateTime<Utc>,
    pub scope: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    scope: Option<String>,
}

impl SoundCloudClient {
    pub fn new(sync: &SyncQueueConfig) -> Result<Self, wreq::Error> {
        Ok(Self {
            http: build_http_client()?,
            api_url: sync.api_url.clone(),
            proxy_url: sync.proxy_url.clone(),
        })
    }

    pub async fn post(
        &self,
        path: &str,
        access_token: &str,
        body: Option<&Value>,
    ) -> Result<Value, SoundCloudError> {
        self.send_api(Method::POST, path, access_token, body).await
    }

    pub async fn put(
        &self,
        path: &str,
        access_token: &str,
        body: Option<&Value>,
    ) -> Result<Value, SoundCloudError> {
        self.send_api(Method::PUT, path, access_token, body).await
    }

    pub async fn delete(&self, path: &str, access_token: &str) -> Result<Value, SoundCloudError> {
        self.send_api(Method::DELETE, path, access_token, None)
            .await
    }

    async fn send_api(
        &self,
        method: Method,
        path: &str,
        access_token: &str,
        body: Option<&Value>,
    ) -> Result<Value, SoundCloudError> {
        let target = self
            .api_url
            .join(path.trim_start_matches('/'))
            .map_err(|_| SoundCloudError::InvalidTokenResponse)?;
        let Some(proxy) = &self.proxy_url else {
            return self
                .send_to(method, &target, None, access_token, body)
                .await;
        };
        match self
            .send_to(method.clone(), &target, Some(proxy), access_token, body)
            .await
        {
            Err(error) if retries_direct(&method, &error) => {
                tracing::debug!(%error, "proxied write failed, sending it directly");
                self.send_to(method, &target, None, access_token, body)
                    .await
            }
            result => result,
        }
    }

    async fn send_to(
        &self,
        method: Method,
        target: &Url,
        proxy: Option<&Url>,
        access_token: &str,
        body: Option<&Value>,
    ) -> Result<Value, SoundCloudError> {
        let mut request = match proxy {
            Some(proxy) => self.http.request(method, proxy.as_str()).header(
                "x-target",
                base64::engine::general_purpose::STANDARD.encode(target.as_str()),
            ),
            None => self.http.request(method, target.as_str()),
        }
        .header(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("OAuth {access_token}"))
                .map_err(|_| SoundCloudError::InvalidTokenResponse)?,
        )
        .header(ACCEPT, "application/json; charset=utf-8")
        .header(ACCEPT_ENCODING, "identity");
        if let Some(body) = body {
            request = request
                .header(CONTENT_TYPE, "application/json; charset=utf-8")
                .json(body);
        }
        let response = request.send().await?;
        let status = response.status();
        let retry_after_seconds = retry_after(response.headers().get(RETRY_AFTER));
        let body = read_body(response).await?;
        if !status.is_success() {
            return Err(api_error(status, body, retry_after_seconds));
        }
        if body.is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned())))
    }
}

fn retries_direct(method: &Method, error: &SoundCloudError) -> bool {
    if error.is_rate_limited() {
        return true;
    }
    if *method == Method::POST {
        return false;
    }
    match error {
        SoundCloudError::Transport(_) => true,
        SoundCloudError::Api { status, .. } => status.is_server_error(),
        SoundCloudError::ResponseTooLarge | SoundCloudError::InvalidTokenResponse => false,
    }
}

impl TokenRefreshClient {
    pub fn new(oauth: &OAuthConfig) -> Result<Self, wreq::Error> {
        Ok(Self {
            http: build_http_client()?,
            token_url: oauth.token_url.clone(),
        })
    }

    pub async fn refresh(
        &self,
        refresh_token: &str,
        client_id: &str,
        client_secret: &str,
    ) -> Result<RefreshedToken, SoundCloudError> {
        let response = self
            .http
            .post(self.token_url.as_str())
            .header(ACCEPT, "application/json; charset=utf-8")
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await?;
        let status = response.status();
        let retry_after_seconds = retry_after(response.headers().get(RETRY_AFTER));
        let body = read_body(response).await?;
        if !status.is_success() {
            return Err(api_error(status, body, retry_after_seconds));
        }
        let response = serde_json::from_slice::<TokenResponse>(&body)
            .map_err(|_| SoundCloudError::InvalidTokenResponse)?;
        let access_token =
            non_empty(response.access_token).ok_or(SoundCloudError::InvalidTokenResponse)?;
        let refresh_token =
            non_empty(response.refresh_token).ok_or(SoundCloudError::InvalidTokenResponse)?;
        let expires_in = response
            .expires_in
            .filter(|seconds| *seconds > 0)
            .ok_or(SoundCloudError::InvalidTokenResponse)?;
        Ok(RefreshedToken {
            expires_at: Utc::now() + chrono::Duration::seconds(expires_in),
            access_token,
            refresh_token,
            scope: non_empty(response.scope),
        })
    }
}

fn build_http_client() -> Result<Client, wreq::Error> {
    sc_fingerprint::builder(None)
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .tcp_keepalive(Duration::from_secs(60))
        .pool_max_idle_per_host(32)
        .build()
}

impl SoundCloudError {
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, Self::Api { status, .. } if *status == StatusCode::UNAUTHORIZED)
    }

    pub fn is_invalid_grant(&self) -> bool {
        let Self::Api { status, body, .. } = self else {
            return false;
        };
        matches!(*status, StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED)
            && oauth_error(body, "invalid_grant")
    }

    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Self::Api { status, .. } if *status == StatusCode::TOO_MANY_REQUESTS)
    }

    pub fn is_app_credentials_error(&self) -> bool {
        let Self::Api { status, body, .. } = self else {
            return false;
        };
        matches!(*status, StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED)
            && (oauth_error(body, "invalid_client") || oauth_error(body, "unauthorized_client"))
    }

    pub fn is_banned(&self) -> bool {
        let Self::Api { status, body, .. } = self else {
            return false;
        };
        *status == StatusCode::FORBIDDEN
            && matches!(body, Value::String(message) if {
                let lower = message.to_lowercase();
                lower.contains("request blocked") || lower.contains("cloudfront")
            })
    }

    pub fn response_excerpt(&self) -> String {
        match self {
            Self::Api { body, .. } => body.to_string().chars().take(400).collect(),
            _ => String::new(),
        }
    }

    pub fn retry_after_seconds(&self) -> Option<i64> {
        match self {
            Self::Api {
                retry_after_seconds,
                ..
            } => *retry_after_seconds,
            _ => None,
        }
    }
}

async fn read_body(response: wreq::Response) -> Result<Vec<u8>, SoundCloudError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(SoundCloudError::ResponseTooLarge);
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(SoundCloudError::ResponseTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn api_error(
    status: StatusCode,
    body: Vec<u8>,
    retry_after_seconds: Option<i64>,
) -> SoundCloudError {
    let body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()))
    };
    SoundCloudError::Api {
        status,
        body,
        retry_after_seconds,
    }
}

fn retry_after(value: Option<&HeaderValue>) -> Option<i64> {
    let value = value?.to_str().ok()?;
    if let Ok(seconds) = value.trim().parse::<i64>() {
        return Some(seconds.max(1));
    }
    let retry_at = DateTime::parse_from_rfc2822(value)
        .ok()?
        .with_timezone(&Utc);
    Some(
        retry_at
            .signed_duration_since(Utc::now())
            .num_seconds()
            .max(1),
    )
}

fn oauth_error(body: &Value, expected: &str) -> bool {
    body.get("error")
        .and_then(Value::as_str)
        .is_some_and(|error| error.eq_ignore_ascii_case(expected))
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(status: StatusCode) -> SoundCloudError {
        SoundCloudError::Api {
            status,
            body: Value::Null,
            retry_after_seconds: None,
        }
    }

    #[test]
    fn a_rate_limited_proxy_sends_every_write_directly() {
        let limited = refused(StatusCode::TOO_MANY_REQUESTS);

        assert!(retries_direct(&Method::PUT, &limited));
        assert!(retries_direct(&Method::POST, &limited));
    }

    #[test]
    fn a_failed_proxy_never_repeats_a_create_and_never_hides_a_refusal() {
        let unavailable = refused(StatusCode::BAD_GATEWAY);

        assert!(retries_direct(&Method::DELETE, &unavailable));
        assert!(!retries_direct(&Method::POST, &unavailable));
        assert!(!retries_direct(
            &Method::PUT,
            &refused(StatusCode::FORBIDDEN)
        ));
    }

    #[test]
    fn invalid_grant_requires_an_explicit_oauth_error() {
        let error = SoundCloudError::Api {
            status: StatusCode::BAD_REQUEST,
            body: serde_json::json!({ "error": "invalid_grant" }),
            retry_after_seconds: None,
        };

        assert!(error.is_invalid_grant());
    }

    #[test]
    fn status_alone_does_not_require_reauthorization() {
        let error = SoundCloudError::Api {
            status: StatusCode::BAD_REQUEST,
            body: Value::Null,
            retry_after_seconds: None,
        };

        assert!(!error.is_invalid_grant());
    }
}
