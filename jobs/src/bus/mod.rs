pub mod advisory;
mod consumer;
#[cfg(test)]
mod reaper_tests;
#[cfg(test)]
mod tests;
pub mod worker_consumers;
#[cfg(test)]
mod worker_tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, ensure};
use async_nats::jetstream::ErrorCode;
use async_nats::jetstream::consumer::{AckPolicy, pull};
use async_nats::jetstream::context::GetStreamErrorKind;
use async_nats::jetstream::object_store::{
    Config as ObjectStoreConfig, DeleteErrorKind, GetErrorKind,
};
use async_nats::jetstream::stream::{
    Config as StreamConfig, DiscardPolicy, RetentionPolicy, StorageType,
};
use async_nats::{Client, ConnectOptions, HeaderMap, Subscriber};
use backend_contracts::{
    IMPRESSION_CONSUMER, IMPRESSION_STREAM, IMPRESSION_SUBJECT, JOB_INGRESS_CONSUMER,
    JOB_INGRESS_STREAM, JOB_INGRESS_SUBJECT,
    pipeline::{
        DEADLINE_HEADER, DONE_EMBED_LYRICS, DONE_ENCODE, DONE_INDEX_AUDIO, DONE_STREAM,
        DONE_TRAIN_COLLAB, DONE_TRAIN_TASTE, DONE_TRANSCRIBE,
        MAX_MESSAGE_BYTES as CONTRACT_MAX_MESSAGE_BYTES, MSG_ID_HEADER, ObjectStoreSpec,
        PIPELINE_STREAMS, PipelineStreamSpec, REPLY_TO_HEADER, STORAGE_EVENTS_STREAM,
        STORAGE_TRACK_REJECTED, STORAGE_TRACK_UPLOADED, StreamDiscard, WORKER_OBJECT_STORES,
    },
};
use futures::StreamExt;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::AsyncReadExt;

use crate::config::NatsConfig;

pub use consumer::{DeliveryContext, WorkConsumer};
pub use worker_consumers::WorkerQueueSnapshot;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(5);
const OBJECT_TRANSFER_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const ACK_WAIT: Duration = Duration::from_secs(2 * 60);
const ACK_PENDING_PER_WORKER: usize = 4;
const DUPLICATE_WINDOW: Duration = Duration::from_secs(2 * 60 * 60);
const MAX_MESSAGE_BYTES: i32 = CONTRACT_MAX_MESSAGE_BYTES as i32;
const RETIRED_STREAMS: &[&str] = &["TRAIN_QUALITY"];
const DEAD_LETTER_STREAM: &str = "JOBS_DEAD_LETTERS";
const DEAD_LETTER_SUBJECTS: &[&str] = &["jobs.dead.>"];
const JOB_DEAD_LETTER_SUBJECT: &str = "jobs.dead.api_background";
const IMPRESSION_DEAD_LETTER_SUBJECT: &str = "jobs.dead.recommendation_impressions";
const COLLAB_RESULT_CONSUMER: &str = "jobs-done-train-collab";
const COLLAB_RESULT_DEAD_LETTER_SUBJECT: &str = "jobs.dead.collab_results";
const TASTE_RESULT_CONSUMER: &str = "jobs-done-train-taste";
const TASTE_RESULT_DEAD_LETTER_SUBJECT: &str = "jobs.dead.taste_results";
const AUDIO_INDEX_RESULT_CONSUMER: &str = "backend-done-index-audio";
const AUDIO_INDEX_RESULT_DEAD_LETTER_SUBJECT: &str = "jobs.dead.audio_index_results";
const LYRICS_EMBEDDING_CONSUMER: &str = "backend-done-embed-lyrics";
const LYRICS_EMBEDDING_DEAD_LETTER_SUBJECT: &str = "jobs.dead.lyrics_embeddings";
const TRANSCRIPTION_RESULT_CONSUMER: &str = "backend-done-transcribe";
const TRANSCRIPTION_RESULT_DEAD_LETTER_SUBJECT: &str = "jobs.dead.transcription_results";
const STORAGE_REJECTION_CONSUMER: &str = "backend-storage-rejected";
const STORAGE_REJECTION_DEAD_LETTER_SUBJECT: &str = "jobs.dead.storage_rejections";
const ENCODE_RESULT_CONSUMER: &str = "backend-done-encode";
const ENCODE_RESULT_DEAD_LETTER_SUBJECT: &str = "jobs.dead.encode_results";
const STORAGE_UPLOAD_CONSUMER: &str = "backend-storage-uploaded";
const STORAGE_UPLOAD_DEAD_LETTER_SUBJECT: &str = "jobs.dead.storage_uploads";

#[derive(Clone)]
pub struct Bus {
    client: Client,
    jetstream: async_nats::jetstream::Context,
    topology: Arc<Topology>,
}

struct Topology {
    streams: Vec<StreamConfig>,
    intact: AtomicBool,
}

impl Topology {
    fn of(config: &NatsConfig) -> Self {
        let owned = [
            work_stream_config(
                JOB_INGRESS_STREAM,
                JOB_INGRESS_SUBJECT,
                config.job_stream_max_bytes,
                config.max_age,
            ),
            work_stream_config(
                IMPRESSION_STREAM,
                IMPRESSION_SUBJECT,
                config.impression_stream_max_bytes,
                config.max_age,
            ),
            dead_letter_stream_config(config),
        ];
        Self {
            streams: owned
                .into_iter()
                .chain(PIPELINE_STREAMS.iter().map(pipeline_stream_config))
                .collect(),
            intact: AtomicBool::new(true),
        }
    }
}

pub struct BusConsumers {
    pub job_ingress: WorkConsumer,
    pub impressions: WorkConsumer,
    pub collab_results: WorkConsumer,
    pub audio_index_results: WorkConsumer,
    pub encode_results: WorkConsumer,
    pub lyrics_embeddings: WorkConsumer,
    pub transcription_results: WorkConsumer,
    pub storage_rejections: WorkConsumer,
    pub storage_uploads: WorkConsumer,
}

#[derive(Debug, thiserror::Error)]
pub enum ObjectStoreError {
    #[error("NATS object {bucket}/{name} does not exist")]
    NotFound { bucket: String, name: String },
    #[error("NATS object name {name} is invalid")]
    InvalidName { name: String },
    #[error("NATS object {bucket}/{name} is {actual} bytes, limit is {limit}")]
    TooLarge {
        bucket: String,
        name: String,
        actual: usize,
        limit: usize,
    },
    #[error("NATS object transfer failed: {0}")]
    Unavailable(#[source] anyhow::Error),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredObject {
    pub name: String,
    pub modified_unix: Option<i64>,
}

impl ObjectStoreError {
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            Self::NotFound { .. } | Self::InvalidName { .. } | Self::TooLarge { .. }
        )
    }
}

#[derive(Deserialize)]
struct RpcReply<T> {
    ok: bool,
    data: Option<T>,
    error: Option<String>,
}

impl Bus {
    pub async fn connect(config: &NatsConfig, instance_id: &str) -> anyhow::Result<Self> {
        let (options, server, display_server) = connection(config, instance_id)?;
        let client = tokio::time::timeout(CONNECT_TIMEOUT, options.connect(server))
            .await
            .context("NATS connection timed out")?
            .context("NATS connection failed")?;
        tracing::info!(server = %display_server, "jobs connected to NATS");

        Ok(Self {
            jetstream: async_nats::jetstream::new(client.clone()),
            client,
            topology: Arc::new(Topology::of(config)),
        })
    }

    pub async fn provision(&self, config: &NatsConfig) -> anyhow::Result<BusConsumers> {
        let job_stream = self
            .ensure_work_stream(
                JOB_INGRESS_STREAM,
                JOB_INGRESS_SUBJECT,
                config.job_stream_max_bytes,
                config.max_age,
            )
            .await?;
        let impression_stream = self
            .ensure_work_stream(
                IMPRESSION_STREAM,
                IMPRESSION_SUBJECT,
                config.impression_stream_max_bytes,
                config.max_age,
            )
            .await?;
        self.ensure_dead_letter_stream(config).await?;
        self.remove_retired_streams(RETIRED_STREAMS).await?;
        self.ensure_pipeline_streams().await?;
        self.ensure_object_stores().await?;
        self.ensure_worker_consumers().await?;

        let job_ingress = self
            .consumer(
                job_stream,
                JOB_INGRESS_CONSUMER,
                JOB_INGRESS_SUBJECT,
                JOB_DEAD_LETTER_SUBJECT,
                config.job_ingress_concurrency,
                config.retry_delay,
            )
            .await?;
        let impressions = self
            .consumer(
                impression_stream,
                IMPRESSION_CONSUMER,
                IMPRESSION_SUBJECT,
                IMPRESSION_DEAD_LETTER_SUBJECT,
                config.impression_concurrency,
                config.retry_delay,
            )
            .await?;
        let done_stream = self
            .jetstream
            .get_stream(DONE_STREAM.name)
            .await
            .context("NATS pipeline result stream could not be loaded")?;
        let collab_results = self
            .consumer(
                done_stream,
                COLLAB_RESULT_CONSUMER,
                DONE_TRAIN_COLLAB,
                COLLAB_RESULT_DEAD_LETTER_SUBJECT,
                1,
                config.retry_delay,
            )
            .await?;
        let lyrics_embeddings = self
            .consumer(
                self.jetstream
                    .get_stream(DONE_STREAM.name)
                    .await
                    .context("NATS pipeline result stream could not be loaded")?,
                LYRICS_EMBEDDING_CONSUMER,
                DONE_EMBED_LYRICS,
                LYRICS_EMBEDDING_DEAD_LETTER_SUBJECT,
                16,
                config.retry_delay,
            )
            .await?;
        let audio_index_results = self
            .consumer(
                self.jetstream
                    .get_stream(DONE_STREAM.name)
                    .await
                    .context("NATS pipeline result stream could not be loaded")?,
                AUDIO_INDEX_RESULT_CONSUMER,
                DONE_INDEX_AUDIO,
                AUDIO_INDEX_RESULT_DEAD_LETTER_SUBJECT,
                16,
                config.retry_delay,
            )
            .await?;
        let encode_results = self
            .consumer(
                self.jetstream
                    .get_stream(DONE_STREAM.name)
                    .await
                    .context("NATS pipeline result stream could not be loaded")?,
                ENCODE_RESULT_CONSUMER,
                DONE_ENCODE,
                ENCODE_RESULT_DEAD_LETTER_SUBJECT,
                16,
                config.retry_delay,
            )
            .await?;
        let transcription_results = self
            .consumer(
                self.jetstream
                    .get_stream(DONE_STREAM.name)
                    .await
                    .context("NATS pipeline result stream could not be loaded")?,
                TRANSCRIPTION_RESULT_CONSUMER,
                DONE_TRANSCRIBE,
                TRANSCRIPTION_RESULT_DEAD_LETTER_SUBJECT,
                16,
                config.retry_delay,
            )
            .await?;
        let storage_rejections = self
            .consumer(
                self.jetstream
                    .get_stream(STORAGE_EVENTS_STREAM.name)
                    .await
                    .context("NATS storage event stream could not be loaded")?,
                STORAGE_REJECTION_CONSUMER,
                STORAGE_TRACK_REJECTED,
                STORAGE_REJECTION_DEAD_LETTER_SUBJECT,
                16,
                config.retry_delay,
            )
            .await?;
        let storage_uploads = self
            .consumer(
                self.jetstream
                    .get_stream(STORAGE_EVENTS_STREAM.name)
                    .await
                    .context("NATS storage event stream could not be loaded")?,
                STORAGE_UPLOAD_CONSUMER,
                STORAGE_TRACK_UPLOADED,
                STORAGE_UPLOAD_DEAD_LETTER_SUBJECT,
                16,
                config.retry_delay,
            )
            .await?;

        Ok(BusConsumers {
            job_ingress,
            impressions,
            collab_results,
            audio_index_results,
            encode_results,
            lyrics_embeddings,
            transcription_results,
            storage_rejections,
            storage_uploads,
        })
    }

    pub async fn taste_results(&self, config: &NatsConfig) -> anyhow::Result<WorkConsumer> {
        let done_stream = self
            .jetstream
            .get_stream(DONE_STREAM.name)
            .await
            .context("NATS pipeline result stream could not be loaded")?;
        self.consumer(
            done_stream,
            TASTE_RESULT_CONSUMER,
            DONE_TRAIN_TASTE,
            TASTE_RESULT_DEAD_LETTER_SUBJECT,
            1,
            config.retry_delay,
        )
        .await
    }

    pub async fn is_available(&self) -> bool {
        if !self.topology.intact.load(Ordering::Relaxed) {
            return false;
        }
        if self.client.connection_state() != async_nats::connection::State::Connected {
            return false;
        }

        matches!(
            tokio::time::timeout(Duration::from_secs(2), self.jetstream.query_account()).await,
            Ok(Ok(_))
        )
    }

    pub async fn subscribe(&self, subject: &'static str) -> anyhow::Result<Subscriber> {
        self.client
            .subscribe(subject)
            .await
            .with_context(|| format!("NATS subscription to {subject} failed"))
    }

    pub async fn publish<T>(&self, subject: &str, payload: &T) -> anyhow::Result<()>
    where
        T: Serialize,
    {
        let payload = serde_json::to_vec(payload).context("NATS payload could not be encoded")?;
        let acknowledgement = tokio::time::timeout(
            PUBLISH_TIMEOUT,
            self.jetstream.publish(subject.to_owned(), payload.into()),
        )
        .await
        .context("NATS publish timed out")?
        .with_context(|| format!("NATS subject {subject} could not be published"))?;
        tokio::time::timeout(PUBLISH_TIMEOUT, acknowledgement)
            .await
            .context("NATS acknowledgement timed out")?
            .with_context(|| format!("NATS subject {subject} was not acknowledged"))?;
        Ok(())
    }

    pub async fn publish_dedup<T>(
        &self,
        subject: &str,
        payload: &T,
        message_id: &str,
    ) -> anyhow::Result<()>
    where
        T: Serialize,
    {
        let payload = serde_json::to_vec(payload).context("NATS payload could not be encoded")?;
        let mut headers = HeaderMap::new();
        headers.insert(MSG_ID_HEADER, message_id);
        let acknowledgement = tokio::time::timeout(
            PUBLISH_TIMEOUT,
            self.jetstream
                .publish_with_headers(subject.to_owned(), headers, payload.into()),
        )
        .await
        .context("NATS publish timed out")?
        .with_context(|| format!("NATS subject {subject} could not be published"))?;
        tokio::time::timeout(PUBLISH_TIMEOUT, acknowledgement)
            .await
            .context("NATS acknowledgement timed out")?
            .with_context(|| format!("NATS subject {subject} was not acknowledged"))?;
        Ok(())
    }

    pub async fn put_object_reader<R>(
        &self,
        bucket: &str,
        name: &str,
        reader: &mut R,
    ) -> Result<(), ObjectStoreError>
    where
        R: tokio::io::AsyncRead + Unpin,
    {
        let store = self
            .jetstream
            .get_object_store(bucket)
            .await
            .map_err(|error| ObjectStoreError::Unavailable(error.into()))?;
        tokio::time::timeout(OBJECT_TRANSFER_TIMEOUT, store.put(name, reader))
            .await
            .map_err(|_| {
                ObjectStoreError::Unavailable(anyhow::anyhow!(
                    "NATS object {bucket}/{name} upload timed out"
                ))
            })?
            .map_err(|error| ObjectStoreError::Unavailable(error.into()))?;
        Ok(())
    }

    pub async fn read_object(
        &self,
        bucket: &str,
        name: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, ObjectStoreError> {
        let store = self
            .jetstream
            .get_object_store(bucket)
            .await
            .map_err(|error| ObjectStoreError::Unavailable(error.into()))?;
        let object = match store.get(name).await {
            Ok(object) => object,
            Err(error) => {
                return Err(match error.kind() {
                    GetErrorKind::NotFound => ObjectStoreError::NotFound {
                        bucket: bucket.to_owned(),
                        name: name.to_owned(),
                    },
                    GetErrorKind::InvalidName => ObjectStoreError::InvalidName {
                        name: name.to_owned(),
                    },
                    _ => ObjectStoreError::Unavailable(error.into()),
                });
            }
        };
        if object.info().size > max_bytes {
            return Err(ObjectStoreError::TooLarge {
                bucket: bucket.to_owned(),
                name: name.to_owned(),
                actual: object.info().size,
                limit: max_bytes,
            });
        }

        let mut payload = Vec::with_capacity(object.info().size);
        let limit = u64::try_from(max_bytes.saturating_add(1)).unwrap_or(u64::MAX);
        tokio::time::timeout(
            OBJECT_TRANSFER_TIMEOUT,
            object.take(limit).read_to_end(&mut payload),
        )
        .await
        .map_err(|_| {
            ObjectStoreError::Unavailable(anyhow::anyhow!(
                "NATS object {bucket}/{name} download timed out"
            ))
        })?
        .map_err(|error| ObjectStoreError::Unavailable(error.into()))?;
        if payload.len() > max_bytes {
            return Err(ObjectStoreError::TooLarge {
                bucket: bucket.to_owned(),
                name: name.to_owned(),
                actual: payload.len(),
                limit: max_bytes,
            });
        }
        Ok(payload)
    }

    pub async fn list_objects(&self, bucket: &str) -> Result<Vec<StoredObject>, ObjectStoreError> {
        let store = self
            .jetstream
            .get_object_store(bucket)
            .await
            .map_err(|error| ObjectStoreError::Unavailable(error.into()))?;
        let listing = async {
            let mut listing = store
                .list()
                .await
                .map_err(|error| ObjectStoreError::Unavailable(error.into()))?;
            let mut objects = Vec::new();
            while let Some(info) = listing.next().await {
                let info = info.map_err(|error| ObjectStoreError::Unavailable(error.into()))?;
                objects.push(StoredObject {
                    name: info.name,
                    modified_unix: info.modified.map(|at| at.unix_timestamp()),
                });
            }
            Ok(objects)
        };
        tokio::time::timeout(OBJECT_TRANSFER_TIMEOUT, listing)
            .await
            .map_err(|_| {
                ObjectStoreError::Unavailable(anyhow::anyhow!(
                    "NATS object listing of {bucket} timed out"
                ))
            })?
    }

    pub async fn delete_object(&self, bucket: &str, name: &str) -> Result<(), ObjectStoreError> {
        let store = self
            .jetstream
            .get_object_store(bucket)
            .await
            .map_err(|error| ObjectStoreError::Unavailable(error.into()))?;
        match store.delete(name).await {
            Ok(()) => Ok(()),
            Err(error) => Err(match error.kind() {
                DeleteErrorKind::NotFound => ObjectStoreError::NotFound {
                    bucket: bucket.to_owned(),
                    name: name.to_owned(),
                },
                DeleteErrorKind::InvalidName => ObjectStoreError::InvalidName {
                    name: name.to_owned(),
                },
                _ => ObjectStoreError::Unavailable(error.into()),
            }),
        }
    }

    pub async fn request<T, R>(
        &self,
        subject: &str,
        payload: &T,
        timeout: Duration,
        msg_id: &str,
    ) -> anyhow::Result<Option<R>>
    where
        T: Serialize,
        R: DeserializeOwned,
    {
        let inbox = self.client.new_inbox();
        let mut subscription = self
            .client
            .subscribe(inbox.clone())
            .await
            .context("NATS reply subscription failed")?;
        subscription
            .unsubscribe_after(1)
            .await
            .context("NATS reply subscription could not be bounded")?;

        let mut headers = HeaderMap::new();
        headers.insert(REPLY_TO_HEADER, inbox.as_str());
        headers.insert(MSG_ID_HEADER, msg_id);
        headers.insert(
            DEADLINE_HEADER,
            deadline_header(SystemTime::now(), timeout).as_str(),
        );
        let payload = serde_json::to_vec(payload).context("NATS payload could not be encoded")?;
        let acknowledgement = tokio::time::timeout(
            PUBLISH_TIMEOUT,
            self.jetstream
                .publish_with_headers(subject.to_owned(), headers, payload.into()),
        )
        .await
        .context("NATS request publish timed out")?
        .with_context(|| format!("NATS request {subject} could not be published"))?;
        tokio::time::timeout(PUBLISH_TIMEOUT, acknowledgement)
            .await
            .context("NATS request acknowledgement timed out")?
            .with_context(|| format!("NATS request {subject} was not acknowledged"))?;

        let Some(message) = tokio::time::timeout(timeout, subscription.next())
            .await
            .context("NATS request timed out")?
        else {
            return Ok(None);
        };
        let reply: RpcReply<R> =
            serde_json::from_slice(&message.payload).context("NATS reply is not valid JSON")?;
        if reply.ok {
            return Ok(reply.data);
        }
        Err(anyhow::anyhow!(
            "NATS request {subject} failed: {}",
            reply.error.unwrap_or_else(|| "unknown error".to_owned())
        ))
    }

    async fn ensure_work_stream(
        &self,
        name: &str,
        subject: &str,
        max_bytes: i64,
        max_age: Duration,
    ) -> anyhow::Result<async_nats::jetstream::stream::Stream> {
        self.ensure_stream(work_stream_config(name, subject, max_bytes, max_age))
            .await
    }

    async fn ensure_dead_letter_stream(&self, config: &NatsConfig) -> anyhow::Result<()> {
        self.ensure_stream(dead_letter_stream_config(config))
            .await?;
        Ok(())
    }

    pub(super) async fn restore_topology(&self) {
        let mut intact = true;
        for desired in &self.topology.streams {
            if let Err(error) = self.restore_stream(desired).await {
                tracing::warn!(
                    stream = %desired.name,
                    error = format!("{error:#}"),
                    "NATS stream could not be restored"
                );
                intact = false;
            }
        }
        for spec in &WORKER_OBJECT_STORES {
            if let Err(error) = self.ensure_object_store(spec).await {
                tracing::warn!(
                    bucket = spec.bucket,
                    error = format!("{error:#}"),
                    "NATS object store could not be restored"
                );
                intact = false;
            }
        }
        if self.topology.intact.swap(intact, Ordering::Relaxed) != intact {
            tracing::warn!(intact, "NATS topology health changed");
        }
    }

    async fn restore_stream(&self, desired: &StreamConfig) -> anyhow::Result<()> {
        match self.jetstream.get_stream(&desired.name).await {
            Ok(stream) if validate_stream(stream.cached_info().config.clone(), desired).is_ok() => {
                return Ok(());
            }
            Ok(_) => {}
            Err(error) if is_missing_stream(&error.kind()) => {
                tracing::warn!(stream = %desired.name, "NATS stream is missing, recreating it");
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("NATS stream {} could not be loaded", desired.name));
            }
        }
        self.ensure_stream(desired.clone()).await?;
        Ok(())
    }

    async fn ensure_pipeline_streams(&self) -> anyhow::Result<()> {
        for spec in PIPELINE_STREAMS {
            self.ensure_stream(pipeline_stream_config(spec)).await?;
        }
        Ok(())
    }

    async fn remove_retired_streams(&self, names: &[&str]) -> anyhow::Result<()> {
        for name in names {
            match self.jetstream.delete_stream(name).await {
                Ok(_) => tracing::info!(stream = name, "retired NATS stream removed"),
                Err(error) if is_missing_stream(&error.kind()) => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("retired NATS stream {name} could not be removed")
                    });
                }
            }
        }
        Ok(())
    }

    async fn ensure_object_stores(&self) -> anyhow::Result<()> {
        for spec in &WORKER_OBJECT_STORES {
            self.ensure_object_store(spec).await?;
        }
        Ok(())
    }

    async fn ensure_object_store(&self, spec: &ObjectStoreSpec) -> anyhow::Result<()> {
        if self.jetstream.get_object_store(spec.bucket).await.is_ok() {
            return Ok(());
        }

        tracing::info!(
            bucket = spec.bucket,
            "NATS object store is missing, creating it"
        );
        let config = ObjectStoreConfig {
            bucket: spec.bucket.to_owned(),
            max_age: spec
                .max_age_seconds
                .map(Duration::from_secs)
                .unwrap_or_default(),
            storage: StorageType::File,
            ..Default::default()
        };
        match self.jetstream.create_object_store(config).await {
            Ok(_) => Ok(()),
            Err(create_error) => self
                .jetstream
                .get_object_store(spec.bucket)
                .await
                .map(|_| ())
                .with_context(|| {
                    format!(
                        "NATS object store {} could not be created: {create_error}",
                        spec.bucket
                    )
                }),
        }
    }

    async fn ensure_stream(
        &self,
        desired: StreamConfig,
    ) -> anyhow::Result<async_nats::jetstream::stream::Stream> {
        let stream = match self
            .jetstream
            .get_or_create_stream(desired.clone())
            .await
        {
            Ok(stream) => stream,
            Err(create_error) => self
                .jetstream
                .get_stream(&desired.name)
                .await
                .map_err(|load_error| {
                    anyhow::anyhow!(
                        "NATS stream {} could not be created ({create_error}) or loaded ({load_error})",
                        desired.name
                    )
                })?,
        };
        let current = stream.cached_info().config.clone();
        let updated = managed_stream_config(current.clone(), &desired);

        if current != updated {
            self.jetstream
                .update_stream(&updated)
                .await
                .with_context(|| format!("NATS stream {} could not be updated", desired.name))?;
        }

        let stream = self
            .jetstream
            .get_stream(&desired.name)
            .await
            .with_context(|| format!("NATS stream {} could not be loaded", desired.name))?;
        validate_stream(stream.cached_info().config.clone(), &desired)?;
        Ok(stream)
    }

    async fn consumer(
        &self,
        stream: async_nats::jetstream::stream::Stream,
        durable: &str,
        filter_subject: &str,
        dead_letter_subject: &str,
        concurrency: usize,
        retry_delay: Duration,
    ) -> anyhow::Result<WorkConsumer> {
        let max_ack_pending = concurrency
            .checked_mul(ACK_PENDING_PER_WORKER)
            .and_then(|value| i64::try_from(value).ok())
            .context("NATS concurrency is too large")?;
        let config = pull::Config {
            durable_name: Some(durable.to_owned()),
            name: Some(durable.to_owned()),
            ack_policy: AckPolicy::Explicit,
            ack_wait: ACK_WAIT,
            max_deliver: -1,
            filter_subject: filter_subject.to_owned(),
            max_ack_pending,
            ..Default::default()
        };
        WorkConsumer::open(
            self.jetstream.clone(),
            stream,
            config,
            dead_letter_subject.to_owned(),
            concurrency,
            retry_delay,
        )
        .await
    }
}

fn work_stream_config(
    name: &str,
    subject: &str,
    max_bytes: i64,
    max_age: Duration,
) -> StreamConfig {
    StreamConfig {
        name: name.to_owned(),
        subjects: vec![subject.to_owned()],
        retention: RetentionPolicy::WorkQueue,
        discard: DiscardPolicy::New,
        storage: StorageType::File,
        max_bytes,
        max_age,
        max_message_size: MAX_MESSAGE_BYTES,
        duplicate_window: DUPLICATE_WINDOW.min(max_age),
        ..Default::default()
    }
}

fn dead_letter_stream_config(config: &NatsConfig) -> StreamConfig {
    let max_age = config.max_age.saturating_mul(10);
    StreamConfig {
        name: DEAD_LETTER_STREAM.to_owned(),
        subjects: DEAD_LETTER_SUBJECTS
            .iter()
            .map(|subject| (*subject).to_owned())
            .collect(),
        retention: RetentionPolicy::Limits,
        discard: DiscardPolicy::New,
        storage: StorageType::File,
        max_bytes: config
            .job_stream_max_bytes
            .min(config.impression_stream_max_bytes),
        max_age,
        max_message_size: MAX_MESSAGE_BYTES,
        duplicate_window: DUPLICATE_WINDOW.min(max_age),
        ..Default::default()
    }
}

fn pipeline_stream_config(spec: &PipelineStreamSpec) -> StreamConfig {
    StreamConfig {
        name: spec.name.to_owned(),
        subjects: spec
            .subjects
            .iter()
            .map(|subject| (*subject).to_owned())
            .collect(),
        retention: if spec.work_queue {
            RetentionPolicy::WorkQueue
        } else {
            RetentionPolicy::Limits
        },
        discard: match spec.discard {
            StreamDiscard::Old => DiscardPolicy::Old,
            StreamDiscard::New => DiscardPolicy::New,
        },
        storage: StorageType::File,
        max_bytes: spec.max_bytes,
        max_age: Duration::from_secs(spec.max_age_seconds),
        max_message_size: MAX_MESSAGE_BYTES,
        duplicate_window: Duration::from_secs(spec.duplicate_window_seconds),
        ..Default::default()
    }
}

fn is_missing_stream(kind: &GetStreamErrorKind) -> bool {
    matches!(
        kind,
        GetStreamErrorKind::JetStream(error) if error.error_code() == ErrorCode::STREAM_NOT_FOUND
    )
}

fn deadline_header(now: SystemTime, timeout: Duration) -> String {
    let deadline = now
        .checked_add(timeout)
        .unwrap_or(now)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    deadline.as_millis().to_string()
}

fn connection(
    config: &NatsConfig,
    instance_id: &str,
) -> anyhow::Result<(ConnectOptions, String, String)> {
    let mut url = url::Url::parse(&config.url).context("NATS_URL is invalid")?;
    let username = (!url.username().is_empty())
        .then(|| urlencoding::decode(url.username()))
        .transpose()
        .context("NATS username is not valid URL encoding")?
        .map(|value| value.into_owned());
    let password = url
        .password()
        .map(urlencoding::decode)
        .transpose()
        .context("NATS password is not valid URL encoding")?
        .map(|value| value.into_owned())
        .unwrap_or_default();
    url.set_username("")
        .map_err(|_| anyhow::anyhow!("NATS_URL username could not be removed"))?;
    url.set_password(None)
        .map_err(|_| anyhow::anyhow!("NATS_URL password could not be removed"))?;

    let display_server = match url.port() {
        Some(port) => format!("{}://{}:{port}", url.scheme(), url.host_str().unwrap_or("")),
        None => format!("{}://{}", url.scheme(), url.host_str().unwrap_or("")),
    };
    let mut options = ConnectOptions::new()
        .name(format!("scd-jobs:{instance_id}"))
        .max_reconnects(None)
        .retry_on_initial_connect();
    if let Some(username) = username {
        options = options.user_and_password(username, password);
    }
    Ok((options, url.to_string(), display_server))
}

fn managed_stream_config(mut current: StreamConfig, desired: &StreamConfig) -> StreamConfig {
    current.subjects.clone_from(&desired.subjects);
    current.retention = desired.retention;
    current.discard = desired.discard;
    current.storage = desired.storage;
    current.max_bytes = desired.max_bytes;
    current.max_age = desired.max_age;
    current.max_message_size = desired.max_message_size;
    current.duplicate_window = desired.duplicate_window;
    current
}

fn validate_stream(current: StreamConfig, expected: &StreamConfig) -> anyhow::Result<()> {
    ensure!(
        managed_stream_config(current.clone(), expected) == current,
        "NATS stream {} configuration does not match the jobs contract",
        expected.name
    );
    Ok(())
}
