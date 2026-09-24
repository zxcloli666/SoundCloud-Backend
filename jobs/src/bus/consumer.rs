use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use async_nats::HeaderMap;
use async_nats::jetstream::AckKind;
use async_nats::jetstream::consumer::{PullConsumer, pull};
use async_nats::jetstream::context::ConsumerInfoErrorKind;
use async_nats::jetstream::stream::{Config as StreamConfig, Stream};
use backend_contracts::Versioned;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::de::DeserializeOwned;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::queue::{JobError, JobResult};

const RECONNECT_DELAY: Duration = Duration::from_secs(2);
const SERVER_DEFAULT_ACK_WAIT: Duration = Duration::from_secs(30);
const PROGRESS_BEATS_PER_ACK_WAIT: u32 = 4;
const MAX_DEAD_LETTER_ERROR_BYTES: usize = 1_000;

pub struct WorkConsumer {
    jetstream: async_nats::jetstream::Context,
    stream: Stream,
    stream_config: StreamConfig,
    config: pull::Config,
    consumer: PullConsumer,
    name: String,
    dead_letter_subject: String,
    concurrency: usize,
    retry_delay: Duration,
}

pub(super) async fn open_work_consumer(
    stream: &Stream,
    config: &pull::Config,
) -> anyhow::Result<PullConsumer> {
    let durable = config
        .durable_name
        .as_deref()
        .context("NATS work consumer needs a durable name")?;
    match stream.consumer_info(durable).await {
        Ok(info) => {
            let mut updated = info.config;
            updated.ack_policy = config.ack_policy;
            updated.ack_wait = config.ack_wait;
            updated.max_deliver = config.max_deliver;
            updated.filter_subject.clone_from(&config.filter_subject);
            updated.max_ack_pending = config.max_ack_pending;
            stream
                .update_consumer(updated)
                .await
                .with_context(|| format!("NATS consumer {durable} could not be updated"))?;
        }
        Err(_) => {
            stream
                .create_consumer(config.clone())
                .await
                .with_context(|| format!("NATS consumer {durable} could not be created"))?;
        }
    };
    stream
        .get_consumer::<pull::Config>(durable)
        .await
        .map_err(|error| anyhow::anyhow!("NATS consumer {durable} could not be loaded: {error}"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeliveryContext {
    pub consumer: String,
    pub stream: String,
    pub stream_sequence: u64,
    pub delivery_attempt: i64,
    pub published_at: DateTime<Utc>,
}

impl WorkConsumer {
    pub(super) async fn open(
        jetstream: async_nats::jetstream::Context,
        stream: Stream,
        config: pull::Config,
        dead_letter_subject: String,
        concurrency: usize,
        retry_delay: Duration,
    ) -> anyhow::Result<Self> {
        let consumer = open_work_consumer(&stream, &config).await?;
        Ok(Self {
            jetstream,
            name: config.durable_name.clone().unwrap_or_default(),
            stream_config: stream.cached_info().config.clone(),
            stream,
            config,
            consumer,
            dead_letter_subject,
            concurrency,
            retry_delay,
        })
    }

    async fn recover(&mut self) {
        match self.stream.consumer_info(&self.name).await {
            Ok(_) => {}
            Err(error) if error.kind() == ConsumerInfoErrorKind::StreamNotFound => {
                self.restore_stream().await;
            }
            Err(error) if error.kind() == ConsumerInfoErrorKind::NotFound => {
                self.reopen().await;
            }
            Err(error) => {
                tracing::debug!(consumer = %self.name, %error, "NATS consumer could not be checked");
            }
        }
    }

    async fn restore_stream(&mut self) {
        match self
            .jetstream
            .get_or_create_stream(self.stream_config.clone())
            .await
        {
            Ok(stream) => {
                tracing::warn!(consumer = %self.name, stream = %self.stream_config.name, "NATS stream was missing and has been recreated");
                self.stream = stream;
                self.reopen().await;
            }
            Err(error) => {
                tracing::warn!(consumer = %self.name, stream = %self.stream_config.name, %error, "NATS stream is missing and could not be recreated yet");
            }
        }
    }

    async fn reopen(&mut self) {
        match open_work_consumer(&self.stream, &self.config).await {
            Ok(consumer) => {
                tracing::warn!(consumer = %self.name, "NATS consumer was missing and has been recreated");
                self.consumer = consumer;
            }
            Err(error) => {
                tracing::warn!(consumer = %self.name, error = format!("{error:#}"), "NATS consumer is missing and could not be recreated yet");
            }
        }
    }

    fn progress_every(&self) -> Duration {
        let ack_wait = if self.config.ack_wait.is_zero() {
            SERVER_DEFAULT_ACK_WAIT
        } else {
            self.config.ack_wait
        };
        ack_wait / PROGRESS_BEATS_PER_ACK_WAIT
    }

    pub async fn run<T, H, F>(
        self,
        cancellation: CancellationToken,
        handler: H,
    ) -> anyhow::Result<()>
    where
        T: DeserializeOwned + Send + 'static,
        H: Fn(T) -> F + Send + Sync + 'static,
        F: Future<Output = JobResult> + Send + 'static,
    {
        self.run_decoded(
            cancellation,
            move |payload, _| handler(payload),
            decode_versioned::<T>,
        )
        .await
    }

    pub async fn run_raw<T, H, F>(
        self,
        cancellation: CancellationToken,
        handler: H,
    ) -> anyhow::Result<()>
    where
        T: DeserializeOwned + Send + 'static,
        H: Fn(T) -> F + Send + Sync + 'static,
        F: Future<Output = JobResult> + Send + 'static,
    {
        self.run_decoded(
            cancellation,
            move |payload, _| handler(payload),
            decode_raw::<T>,
        )
        .await
    }

    pub async fn run_raw_with_context<T, H, F>(
        self,
        cancellation: CancellationToken,
        handler: H,
    ) -> anyhow::Result<()>
    where
        T: DeserializeOwned + Send + 'static,
        H: Fn(T, DeliveryContext) -> F + Send + Sync + 'static,
        F: Future<Output = JobResult> + Send + 'static,
    {
        self.run_decoded(cancellation, handler, decode_raw::<T>)
            .await
    }

    async fn run_decoded<T, H, F>(
        mut self,
        cancellation: CancellationToken,
        handler: H,
        decode: fn(&[u8]) -> serde_json::Result<T>,
    ) -> anyhow::Result<()>
    where
        T: DeserializeOwned + Send + 'static,
        H: Fn(T, DeliveryContext) -> F + Send + Sync + 'static,
        F: Future<Output = JobResult> + Send + 'static,
    {
        let handler = Arc::new(handler);
        let permits = Arc::new(Semaphore::new(self.concurrency));
        let mut tasks = JoinSet::new();

        loop {
            observe_ready(&mut tasks)?;
            if cancellation.is_cancelled() {
                break;
            }

            let mut messages = match self.consumer.messages().await {
                Ok(messages) => messages,
                Err(error) => {
                    tracing::warn!(consumer = %self.name, %error, "NATS pull stream unavailable");
                    wait_for_reconnect(&cancellation).await;
                    self.recover().await;
                    continue;
                }
            };

            loop {
                observe_ready(&mut tasks)?;
                let delivery = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => break,
                    delivery = messages.next() => delivery,
                };
                let message = match delivery {
                    Some(Ok(message)) => message,
                    Some(Err(error)) => {
                        tracing::warn!(consumer = %self.name, %error, "NATS pull stream interrupted");
                        break;
                    }
                    None => break,
                };

                let worker = DeliveryWorker {
                    jetstream: self.jetstream.clone(),
                    consumer: self.name.clone(),
                    dead_letter_subject: self.dead_letter_subject.clone(),
                    retry_delay: self.retry_delay,
                    progress_every: self.progress_every(),
                    permits: permits.clone(),
                    cancellation: cancellation.clone(),
                };
                let handler = handler.clone();
                tasks.spawn(async move {
                    worker.process::<T, H, F>(message, handler, decode).await;
                });
            }

            if !cancellation.is_cancelled() {
                wait_for_reconnect(&cancellation).await;
                self.recover().await;
            }
        }

        while let Some(result) = tasks.join_next().await {
            result?;
        }
        Ok(())
    }
}

struct DeliveryWorker {
    jetstream: async_nats::jetstream::Context,
    consumer: String,
    dead_letter_subject: String,
    retry_delay: Duration,
    progress_every: Duration,
    permits: Arc<Semaphore>,
    cancellation: CancellationToken,
}

impl DeliveryWorker {
    async fn process<T, H, F>(
        &self,
        message: async_nats::jetstream::Message,
        handler: Arc<H>,
        decode: fn(&[u8]) -> serde_json::Result<T>,
    ) where
        T: DeserializeOwned + Send + 'static,
        H: Fn(T, DeliveryContext) -> F + Send + Sync + 'static,
        F: Future<Output = JobResult> + Send + 'static,
    {
        let context = match delivery_context(&message) {
            Ok(context) => context,
            Err(error) => {
                self.retry(message, JobError::retryable(error)).await;
                return;
            }
        };
        let payload = match decode(&message.payload) {
            Ok(payload) => payload,
            Err(error) => {
                self.finish_permanent(message, JobError::permanent(error))
                    .await;
                return;
            }
        };

        let Some(_permit) = self
            .run_with_progress(&message, self.wait_for_capacity())
            .await
        else {
            self.hand_back(message).await;
            return;
        };
        match self
            .run_with_progress(&message, handler(payload, context))
            .await
        {
            Ok(()) => acknowledge(&self.consumer, message).await,
            Err(error) if error.is_retryable() => self.retry(message, error).await,
            Err(error) => self.finish_permanent(message, error).await,
        }
    }

    async fn wait_for_capacity(&self) -> Option<OwnedSemaphorePermit> {
        tokio::select! {
            biased;
            _ = self.cancellation.cancelled() => None,
            permit = self.permits.clone().acquire_owned() => permit.ok(),
        }
    }

    async fn run_with_progress<F>(
        &self,
        message: &async_nats::jetstream::Message,
        future: F,
    ) -> F::Output
    where
        F: Future,
    {
        let progress = tokio::time::sleep(self.progress_every);
        tokio::pin!(future);
        tokio::pin!(progress);

        loop {
            tokio::select! {
                result = &mut future => return result,
                _ = &mut progress => {
                    if let Err(error) = message.ack_with(AckKind::Progress).await {
                        tracing::warn!(consumer = %self.consumer, %error, "NATS progress acknowledgement failed");
                    }
                    progress.as_mut().reset(tokio::time::Instant::now() + self.progress_every);
                }
            }
        }
    }

    async fn hand_back(&self, message: async_nats::jetstream::Message) {
        if let Err(error) = message.ack_with(AckKind::Nak(None)).await {
            tracing::debug!(consumer = %self.consumer, %error, "NATS delivery could not be handed back at shutdown");
        }
    }

    async fn retry(&self, message: async_nats::jetstream::Message, error: JobError) {
        tracing::warn!(consumer = %self.consumer, %error, "NATS delivery will be retried");
        if let Err(ack_error) = message.ack_with(AckKind::Nak(Some(self.retry_delay))).await {
            tracing::warn!(consumer = %self.consumer, error = %ack_error, "NATS retry acknowledgement failed");
        }
    }

    async fn finish_permanent(&self, message: async_nats::jetstream::Message, error: JobError) {
        tracing::error!(consumer = %self.consumer, %error, "NATS delivery is permanently invalid");
        match self.publish_dead_letter(&message, &error.to_string()).await {
            Ok(()) => {
                if let Err(ack_error) = message.ack_with(AckKind::Term).await {
                    tracing::warn!(consumer = %self.consumer, error = %ack_error, "NATS terminal acknowledgement failed");
                }
            }
            Err(publish_error) => {
                tracing::error!(consumer = %self.consumer, error = %publish_error, "NATS dead letter publish failed");
                if let Err(ack_error) = message.ack_with(AckKind::Nak(Some(self.retry_delay))).await
                {
                    tracing::warn!(consumer = %self.consumer, error = %ack_error, "NATS delivery could not be returned after the dead letter failed");
                }
            }
        }
    }

    async fn publish_dead_letter(
        &self,
        message: &async_nats::jetstream::Message,
        error: &str,
    ) -> anyhow::Result<()> {
        let mut headers = HeaderMap::new();
        headers.insert("X-Original-Subject", message.subject.as_str());
        headers.insert("X-Delivery-Error", header_value(error));
        if let Ok(info) = message.info() {
            headers.insert(
                "Nats-Msg-Id",
                format!(
                    "{}:{}:{}:{}",
                    self.consumer,
                    info.stream,
                    info.stream_sequence,
                    info.published.unix_timestamp_nanos()
                )
                .as_str(),
            );
        }

        let acknowledgement = self
            .jetstream
            .publish_with_headers(
                self.dead_letter_subject.clone(),
                headers,
                message.payload.clone(),
            )
            .await?;
        acknowledgement.await?;
        Ok(())
    }
}

fn delivery_context(message: &async_nats::jetstream::Message) -> anyhow::Result<DeliveryContext> {
    let info = message
        .info()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let published_at = DateTime::<Utc>::from_timestamp(
        info.published.unix_timestamp(),
        info.published.nanosecond(),
    )
    .ok_or_else(|| anyhow::anyhow!("NATS publish timestamp is out of range"))?;
    Ok(DeliveryContext {
        consumer: info.consumer.to_owned(),
        stream: info.stream.to_owned(),
        stream_sequence: info.stream_sequence,
        delivery_attempt: info.delivered,
        published_at,
    })
}

fn decode_versioned<T>(payload: &[u8]) -> serde_json::Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_slice::<Versioned<T>>(payload).map(Versioned::into_latest)
}

fn decode_raw<T>(payload: &[u8]) -> serde_json::Result<T>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(payload)
}

async fn acknowledge(consumer: &str, message: async_nats::jetstream::Message) {
    if let Err(error) = message.ack().await {
        tracing::warn!(consumer, %error, "NATS acknowledgement failed");
    }
}

fn observe_ready(tasks: &mut JoinSet<()>) -> anyhow::Result<()> {
    while let Some(result) = tasks.try_join_next() {
        result?;
    }
    Ok(())
}

async fn wait_for_reconnect(cancellation: &CancellationToken) {
    tokio::select! {
        _ = cancellation.cancelled() => {}
        _ = tokio::time::sleep(RECONNECT_DELAY) => {}
    }
}

fn truncate(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_owned();
    }

    let end = (0..=maximum_bytes)
        .rev()
        .find(|end| value.is_char_boundary(*end))
        .unwrap_or_default();
    value.get(..end).unwrap_or_default().to_owned()
}

fn header_value(value: &str) -> String {
    let sanitized = value.replace(['\r', '\n'], " ");
    truncate(&sanitized, MAX_DEAD_LETTER_ERROR_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dead_letter_errors_are_truncated_on_character_boundaries() {
        assert_eq!(truncate("абв", 3), "а");
        assert_eq!(truncate("a🎵b", 4), "a");
        assert_eq!(truncate("🎵", 3), "");
        assert_eq!(truncate("short", 0), "");
        assert_eq!(truncate("short", 5), "short");
    }

    #[test]
    fn dead_letter_headers_cannot_inject_lines() {
        assert_eq!(header_value("first\r\nsecond"), "first  second");
    }
}
