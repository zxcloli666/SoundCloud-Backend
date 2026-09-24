use std::time::Duration;

use base64::Engine;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde_json::Value;
use wreq::header::{ACCEPT, ACCEPT_ENCODING, AUTHORIZATION, HeaderValue, RETRY_AFTER};
use wreq::{Client, StatusCode, Url};

use crate::config::SyncQueueConfig;

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_RETRY_AFTER_SECONDS: i64 = 24 * 60 * 60;

pub struct PlaylistReadClient {
    http: Client,
    api_url: Url,
    proxy_url: Option<Url>,
}

pub struct PlaylistReadResponse {
    pub value: Value,
    pub body_bytes: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum PlaylistReadError {
    #[error("SoundCloud playlist read failed: {0}")]
    Transport(#[from] wreq::Error),

    #[error("SoundCloud playlist read returned HTTP {status}")]
    Api {
        status: StatusCode,
        body: Value,
        retry_after_seconds: Option<i64>,
    },

    #[error("SoundCloud playlist response exceeded the size limit")]
    ResponseTooLarge,

    #[error("SoundCloud playlist pagination target is invalid")]
    InvalidTarget,

    #[error("SoundCloud playlist response is not valid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
}

impl PlaylistReadClient {
    pub fn new(config: &SyncQueueConfig) -> Result<Self, wreq::Error> {
        let http = sc_fingerprint::builder(None)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .tcp_keepalive(Duration::from_secs(60))
            .pool_max_idle_per_host(16)
            .build()?;
        Ok(Self {
            http,
            api_url: config.api_url.clone(),
            proxy_url: config.proxy_url.clone(),
        })
    }

    pub async fn get_path(
        &self,
        path: &str,
        access_token: &str,
    ) -> Result<PlaylistReadResponse, PlaylistReadError> {
        let target = self
            .api_url
            .join(path.trim_start_matches('/'))
            .map_err(|_| PlaylistReadError::InvalidTarget)?;
        self.get(target, access_token).await
    }

    pub async fn get_next(
        &self,
        target: &str,
        access_token: &str,
    ) -> Result<PlaylistReadResponse, PlaylistReadError> {
        let target = Url::parse(target).map_err(|_| PlaylistReadError::InvalidTarget)?;
        if !same_origin(&self.api_url, &target)
            || !target.username().is_empty()
            || target.password().is_some()
            || target.fragment().is_some()
        {
            return Err(PlaylistReadError::InvalidTarget);
        }
        self.get(target, access_token).await
    }

    async fn get(
        &self,
        target: Url,
        access_token: &str,
    ) -> Result<PlaylistReadResponse, PlaylistReadError> {
        let request = match &self.proxy_url {
            Some(proxy) => self.http.get(proxy.clone()).header(
                "x-target",
                base64::engine::general_purpose::STANDARD.encode(target.as_str()),
            ),
            None => self.http.get(target),
        }
        .header(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("OAuth {access_token}"))
                .map_err(|_| PlaylistReadError::InvalidTarget)?,
        )
        .header(ACCEPT, "application/json; charset=utf-8")
        .header(ACCEPT_ENCODING, "identity");
        let response = request.send().await?;
        let status = response.status();
        let retry_after_seconds = retry_after(response.headers().get(RETRY_AFTER));
        let body = read_body(response).await?;
        if !status.is_success() {
            return Err(api_error(status, body, retry_after_seconds));
        }
        Ok(PlaylistReadResponse {
            value: serde_json::from_slice(&body)?,
            body_bytes: body.len(),
        })
    }
}

impl PlaylistReadError {
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, Self::Api { status, .. } if *status == StatusCode::UNAUTHORIZED)
    }
}

async fn read_body(response: wreq::Response) -> Result<Vec<u8>, PlaylistReadError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(PlaylistReadError::ResponseTooLarge);
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(PlaylistReadError::ResponseTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn api_error(
    status: StatusCode,
    body: Vec<u8>,
    retry_after_seconds: Option<i64>,
) -> PlaylistReadError {
    let body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()))
    };
    PlaylistReadError::Api {
        status,
        body,
        retry_after_seconds,
    }
}

fn retry_after(value: Option<&HeaderValue>) -> Option<i64> {
    let value = value?.to_str().ok()?;
    if let Ok(seconds) = value.trim().parse::<i64>() {
        return Some(seconds.clamp(1, MAX_RETRY_AFTER_SECONDS));
    }
    let retry_at = DateTime::parse_from_rfc2822(value)
        .ok()?
        .with_timezone(&Utc);
    Some(
        retry_at
            .signed_duration_since(Utc::now())
            .num_seconds()
            .clamp(1, MAX_RETRY_AFTER_SECONDS),
    )
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagination_target_must_keep_the_api_origin() {
        let api = Url::parse("https://api.soundcloud.com/").unwrap();

        assert!(same_origin(
            &api,
            &Url::parse("https://api.soundcloud.com/playlists/42/tracks?offset=200").unwrap()
        ));
        assert!(!same_origin(
            &api,
            &Url::parse("https://attacker.example/playlists/42/tracks").unwrap()
        ));
        assert!(!same_origin(
            &api,
            &Url::parse("http://api.soundcloud.com/playlists/42/tracks").unwrap()
        ));
    }

    #[test]
    fn retry_after_is_bounded_for_numeric_and_date_values() {
        let numeric = HeaderValue::from_static("9223372036854775807");
        let date = HeaderValue::from_static("Fri, 31 Dec 9999 23:59:59 GMT");

        assert_eq!(retry_after(Some(&numeric)), Some(MAX_RETRY_AFTER_SECONDS));
        assert_eq!(retry_after(Some(&date)), Some(MAX_RETRY_AFTER_SECONDS));
    }
}
