use std::sync::Arc;

use async_nats::{ConnectOptions, HeaderMap};
use bytes::Bytes;
use serde::Serialize;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::error::{AppError, AppResult};

#[derive(Clone)]
pub struct NatsService {
    js: async_nats::jetstream::Context,
}

impl NatsService {
    pub async fn connect(url: &str, _shutdown: CancellationToken) -> AppResult<Arc<Self>> {
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
        info!(url = %clean, "NATS connected");
        Ok(Arc::new(Self {
            js: async_nats::jetstream::new(client),
        }))
    }

    pub async fn subscribe(&self, subject: &str) -> AppResult<async_nats::Subscriber> {
        self.js
            .client()
            .subscribe(subject.to_owned())
            .await
            .map_err(|error| AppError::internal(format!("nats subscribe {subject}: {error}")))
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
