use std::sync::Arc;
use std::time::Duration;

use async_nats::client::RequestErrorKind;
use async_nats::{ConnectOptions, HeaderMap};
use bytes::Bytes;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tls_common::ClientIdentity;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::error::{AppError, AppResult};

#[derive(Debug, thiserror::Error)]
pub enum NatsRequestError {
    #[error("nats request {0} timed out")]
    Timeout(String),
    #[error("nats request {0} has no responders")]
    NoResponders(String),
    #[error("nats request {0} failed: {1}")]
    Transport(String, String),
    #[error("nats reply to {0} did not decode: {1}")]
    Decode(String, String),
}

#[derive(Clone)]
pub struct NatsService {
    js: async_nats::jetstream::Context,
}

impl NatsService {
    #[cfg(test)]
    pub async fn connect(url: &str, shutdown: CancellationToken) -> AppResult<Arc<Self>> {
        Self::connect_with(url, None, shutdown).await
    }

    pub async fn connect_with(
        url: &str,
        tls: Option<&ClientIdentity>,
        _shutdown: CancellationToken,
    ) -> AppResult<Arc<Self>> {
        let parsed = url::Url::parse(url)
            .map_err(|error| AppError::internal(format!("invalid NATS_URL: {error}")))?;
        let user = parsed.username();
        let password = parsed.password().unwrap_or("");
        let clean = format!(
            "{}://{}{}",
            parsed.scheme(),
            parsed.host_str().unwrap_or("localhost"),
            parsed
                .port()
                .map(|port| format!(":{port}"))
                .unwrap_or_default()
        );
        let mut options = ConnectOptions::new()
            .name("backend")
            .max_reconnects(None)
            .retry_on_initial_connect();
        if let Some(tls) = tls {
            options = options
                .require_tls(true)
                .add_root_certificates(tls.ca.clone())
                .add_client_certificate(tls.cert.clone(), tls.key.clone());
        }
        if !user.is_empty() {
            let user = urlencoding::decode(user)
                .map_err(|error| AppError::internal(format!("nats user decode: {error}")))?
                .into_owned();
            let password = urlencoding::decode(password)
                .map_err(|error| AppError::internal(format!("nats pass decode: {error}")))?
                .into_owned();
            options = options.user_and_password(user, password);
        }
        let client = options
            .connect(clean.as_str())
            .await
            .map_err(|error| AppError::internal(format!("NATS connect failed: {error}")))?;
        info!(url = %clean, "NATS client configured");
        Ok(Arc::new(Self {
            js: async_nats::jetstream::new(client),
        }))
    }

    #[allow(dead_code)]
    pub async fn request<P, R>(
        &self,
        subject: &str,
        payload: &P,
        timeout: Duration,
    ) -> Result<R, NatsRequestError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let body = serde_json::to_vec(payload)
            .map_err(|error| NatsRequestError::Transport(subject.to_owned(), error.to_string()))?;
        let request = async_nats::Request::new()
            .payload(Bytes::from(body))
            .timeout(Some(timeout));
        let reply = self
            .js
            .client()
            .send_request(subject.to_owned(), request)
            .await
            .map_err(|error| match error.kind() {
                RequestErrorKind::TimedOut => NatsRequestError::Timeout(subject.to_owned()),
                RequestErrorKind::NoResponders => {
                    NatsRequestError::NoResponders(subject.to_owned())
                }
                _ => NatsRequestError::Transport(subject.to_owned(), error.to_string()),
            })?;
        serde_json::from_slice(&reply.payload)
            .map_err(|error| NatsRequestError::Decode(subject.to_owned(), error.to_string()))
    }

    pub async fn subscribe(&self, subject: &str) -> AppResult<async_nats::Subscriber> {
        self.js
            .client()
            .subscribe(subject.to_owned())
            .await
            .map_err(|error| AppError::internal(format!("nats subscribe {subject}: {error}")))
    }

    pub async fn publish_core(&self, subject: &str, payload: Bytes) -> AppResult<()> {
        self.js
            .client()
            .publish(subject.to_owned(), payload)
            .await
            .map_err(|error| AppError::internal(format!("nats publish {subject}: {error}")))
    }

    pub async fn publish_dedup<P>(
        &self,
        subject: &str,
        payload: &P,
        message_id: &str,
    ) -> AppResult<()>
    where
        P: Serialize,
    {
        let mut headers = HeaderMap::new();
        headers.insert("Nats-Msg-Id", message_id);
        let body = Bytes::from(
            serde_json::to_vec(payload)
                .map_err(|error| AppError::internal(format!("publish encode: {error}")))?,
        );
        let acknowledgement = self
            .js
            .publish_with_headers(subject.to_owned(), headers, body)
            .await
            .map_err(|error| AppError::internal(format!("jetstream publish {subject}: {error}")))?;
        acknowledgement
            .await
            .map_err(|error| AppError::internal(format!("jetstream ack {subject}: {error}")))?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "nats_tests.rs"]
mod tests;
