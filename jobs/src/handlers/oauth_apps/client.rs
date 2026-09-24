use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::StreamExt;
use wreq::header::RETRY_AFTER;
use wreq::{Client, StatusCode};

use crate::config::OAuthConfig;

use super::model::{ClaimedApp, Token, TokenResponse};

const MAX_RESPONSE_BYTES: usize = 64 * 1024;

pub enum TokenRequestOutcome {
    RefreshSuccess(Token),
    ClientCredentialsSuccess(Token),
    Rejected(Rejection),
    Incomplete,
    Unavailable,
}

pub struct Rejection {
    pub kind: RejectionKind,
    pub retry_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectionKind {
    InvalidGrant,
    AppCredentials,
    RateLimited,
    Upstream,
    Request,
}

impl Rejection {
    pub fn invalid_grant(&self) -> bool {
        self.kind == RejectionKind::InvalidGrant
    }

    pub fn shared_cooldown(&self) -> bool {
        matches!(
            self.kind,
            RejectionKind::AppCredentials | RejectionKind::RateLimited | RejectionKind::Upstream
        )
    }

    pub fn minimum_cooldown_seconds(&self) -> i64 {
        match self.kind {
            RejectionKind::AppCredentials => 30 * 60,
            RejectionKind::RateLimited => 5 * 60,
            RejectionKind::Upstream => 30,
            RejectionKind::InvalidGrant | RejectionKind::Request => 1,
        }
    }
}

pub struct OAuthTokenClient {
    http: Client,
    token_url: wreq::Url,
}

impl OAuthTokenClient {
    pub fn new(config: &OAuthConfig) -> Result<Self, wreq::Error> {
        let http = sc_fingerprint::builder(None)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            token_url: config.token_url.clone(),
        })
    }

    pub async fn refresh(&self, app: &ClaimedApp, refresh_token: &str) -> TokenRequestOutcome {
        self.send_refresh(self.http.post(self.token_url.clone()).form(&[
            ("grant_type", "refresh_token"),
            ("client_id", app.client_id.as_str()),
            ("client_secret", app.client_secret.as_str()),
            ("refresh_token", refresh_token),
        ]))
        .await
    }

    pub async fn client_credentials(&self, app: &ClaimedApp) -> TokenRequestOutcome {
        self.send_client_credentials(
            self.http
                .post(self.token_url.clone())
                .basic_auth(&app.client_id, Some(&app.client_secret))
                .form(&[("grant_type", "client_credentials")]),
        )
        .await
    }

    async fn send_refresh(&self, request: wreq::RequestBuilder) -> TokenRequestOutcome {
        self.send(
            request,
            Token::from_refresh,
            TokenRequestOutcome::RefreshSuccess,
        )
        .await
    }

    async fn send_client_credentials(&self, request: wreq::RequestBuilder) -> TokenRequestOutcome {
        self.send(
            request,
            Token::from_client_credentials,
            TokenRequestOutcome::ClientCredentialsSuccess,
        )
        .await
    }

    async fn send(
        &self,
        request: wreq::RequestBuilder,
        decode: fn(TokenResponse) -> Result<Token, ()>,
        success: fn(Token) -> TokenRequestOutcome,
    ) -> TokenRequestOutcome {
        let response = match request.send().await {
            Ok(response) => response,
            Err(_) => return TokenRequestOutcome::Unavailable,
        };
        let retry_at = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(parse_retry_after);
        let status = response.status();
        let body = match read_body(response).await {
            Ok(body) => body,
            Err(_) if status.is_success() => return TokenRequestOutcome::Unavailable,
            Err(_) => {
                return TokenRequestOutcome::Rejected(Rejection {
                    kind: rejection_kind(status, &[]),
                    retry_at,
                });
            }
        };
        if !status.is_success() {
            return TokenRequestOutcome::Rejected(Rejection {
                kind: rejection_kind(status, &body),
                retry_at,
            });
        }
        match serde_json::from_slice::<TokenResponse>(&body) {
            Ok(token) => decode(token).map_or(TokenRequestOutcome::Incomplete, success),
            Err(_) => TokenRequestOutcome::Incomplete,
        }
    }
}

async fn read_body(response: wreq::Response) -> Result<Vec<u8>, ()> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(());
    }
    let mut bytes = Vec::new();
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|_| ())?;
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn rejection_kind(status: StatusCode, body: &[u8]) -> RejectionKind {
    let error = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    if error
        .as_deref()
        .is_some_and(|error| error.eq_ignore_ascii_case("invalid_grant"))
    {
        return RejectionKind::InvalidGrant;
    }
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
        || error.as_deref().is_some_and(|error| {
            error.eq_ignore_ascii_case("invalid_client")
                || error.eq_ignore_ascii_case("unauthorized_client")
        })
    {
        return RejectionKind::AppCredentials;
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return RejectionKind::RateLimited;
    }
    if status.is_server_error() {
        return RejectionKind::Upstream;
    }
    RejectionKind::Request
}

fn parse_retry_after(value: &str) -> Option<DateTime<Utc>> {
    value
        .parse::<i64>()
        .ok()
        .filter(|seconds| *seconds >= 0)
        .and_then(|seconds| Utc::now().checked_add_signed(chrono::Duration::seconds(seconds)))
        .or_else(|| {
            DateTime::parse_from_rfc2822(value)
                .ok()
                .map(|date| date.with_timezone(&Utc))
        })
}

#[cfg(test)]
mod tests;
