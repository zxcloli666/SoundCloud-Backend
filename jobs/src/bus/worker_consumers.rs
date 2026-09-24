use std::time::Duration;

use anyhow::Context;
use async_nats::jetstream::ErrorCode;
use async_nats::jetstream::consumer::{self, AckPolicy, DeliverPolicy, ReplayPolicy, pull};
use async_nats::jetstream::context::ConsumerInfoErrorKind;
use async_nats::jetstream::stream::{ConsumerUpdateErrorKind, Stream};
use backend_contracts::pipeline::WORKER_STREAMS;
use backend_contracts::worker_contract::{WORKER_LANES, WorkerLane, WorkerLaneSpec};

use super::{Bus, is_missing_stream, pipeline_stream_config};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConsumerProvision {
    Unchanged,
    Created,
    Updated,
    Recreated,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConsumerBacklog {
    pub lane: WorkerLane,
    pub durable: &'static str,
    pub pending: u64,
    pub waiting: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StreamFill {
    pub stream: &'static str,
    pub ratio: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorkerQueueSnapshot {
    pub consumers: Vec<ConsumerBacklog>,
    pub streams: Vec<StreamFill>,
}

impl Bus {
    pub async fn ensure_worker_consumers(&self) -> anyhow::Result<()> {
        for spec in &WORKER_LANES {
            let provision = self.ensure_worker_consumer(spec).await?;
            if provision != ConsumerProvision::Unchanged {
                tracing::info!(
                    lane = spec.lane.as_str(),
                    durable = spec.durable,
                    ?provision,
                    "worker consumer brought to the contract"
                );
            }
        }
        Ok(())
    }

    pub(super) async fn ensure_worker_consumer(
        &self,
        spec: &WorkerLaneSpec,
    ) -> anyhow::Result<ConsumerProvision> {
        let stream = match self.jetstream.get_stream(spec.stream.name).await {
            Ok(stream) => stream,
            Err(error) if is_missing_stream(&error.kind()) => {
                tracing::warn!(
                    lane = spec.lane.as_str(),
                    stream = spec.stream.name,
                    "NATS worker stream is missing, recreating it"
                );
                self.ensure_stream(pipeline_stream_config(&spec.stream))
                    .await?
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("NATS stream {} could not be loaded", spec.stream.name)
                });
            }
        };
        let desired = worker_consumer_config(spec)?;
        let current = match stream.consumer_info(spec.durable).await {
            Ok(info) => info.config,
            Err(error) if error.kind() == ConsumerInfoErrorKind::NotFound => {
                stream.create_consumer(desired).await.with_context(|| {
                    format!("NATS worker consumer {} could not be created", spec.durable)
                })?;
                return Ok(ConsumerProvision::Created);
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("NATS worker consumer {} could not be read", spec.durable)
                });
            }
        };
        if follows_contract(&current, &desired) {
            return Ok(ConsumerProvision::Unchanged);
        }

        match stream
            .update_consumer(with_contract_fields(current, &desired))
            .await
        {
            Ok(_) => Ok(ConsumerProvision::Updated),
            Err(error) if refuses_in_place_update(&error.kind()) => {
                tracing::warn!(
                    lane = spec.lane.as_str(),
                    durable = spec.durable,
                    %error,
                    "NATS refused to update the worker consumer in place, recreating it"
                );
                recreate(&stream, spec, desired).await?;
                crate::metrics::record_worker_consumer_recreated(spec.lane);
                Ok(ConsumerProvision::Recreated)
            }
            Err(error) => Err(error).with_context(|| {
                format!("NATS worker consumer {} could not be updated", spec.durable)
            }),
        }
    }

    pub async fn worker_queue_snapshot(&self) -> WorkerQueueSnapshot {
        let mut snapshot = WorkerQueueSnapshot::default();
        for spec in &WORKER_STREAMS {
            match self.jetstream.get_stream(spec.name).await {
                Ok(stream) => snapshot.streams.push(StreamFill {
                    stream: spec.name,
                    ratio: fill_ratio(
                        stream.cached_info().state.bytes,
                        stream.cached_info().config.max_bytes,
                    ),
                }),
                Err(error) => {
                    tracing::debug!(stream = spec.name, %error, "worker stream is not readable");
                }
            }
        }
        for spec in &WORKER_LANES {
            let info = match self.jetstream.get_stream(spec.stream.name).await {
                Ok(stream) => stream.consumer_info(spec.durable).await.map_err(Into::into),
                Err(error) => Err(anyhow::Error::from(error)),
            };
            match info {
                Ok(info) => snapshot.consumers.push(ConsumerBacklog {
                    lane: spec.lane,
                    durable: spec.durable,
                    pending: info.num_pending,
                    waiting: info.num_waiting,
                }),
                Err(error) => {
                    tracing::debug!(durable = spec.durable, %error, "worker consumer is not readable");
                }
            }
        }
        snapshot
    }
}

async fn recreate(
    stream: &Stream,
    spec: &WorkerLaneSpec,
    desired: pull::Config,
) -> anyhow::Result<()> {
    stream
        .delete_consumer(spec.durable)
        .await
        .with_context(|| format!("NATS worker consumer {} could not be deleted", spec.durable))?;
    stream.create_consumer(desired).await.with_context(|| {
        format!(
            "NATS worker consumer {} could not be recreated",
            spec.durable
        )
    })?;
    Ok(())
}

pub(super) fn worker_consumer_config(spec: &WorkerLaneSpec) -> anyhow::Result<pull::Config> {
    Ok(pull::Config {
        durable_name: Some(spec.durable.to_owned()),
        name: Some(spec.durable.to_owned()),
        filter_subject: spec.filter_subject.to_owned(),
        ack_policy: AckPolicy::Explicit,
        deliver_policy: DeliverPolicy::All,
        replay_policy: ReplayPolicy::Instant,
        ack_wait: Duration::from_secs(spec.ack_wait_s),
        max_deliver: i64::try_from(spec.max_deliver())
            .context("worker max_deliver does not fit NATS")?,
        max_ack_pending: i64::try_from(spec.max_ack_pending)
            .context("worker max_ack_pending does not fit NATS")?,
        ..Default::default()
    })
}

pub(super) fn with_contract_fields(
    mut current: consumer::Config,
    desired: &pull::Config,
) -> consumer::Config {
    current.filter_subject.clone_from(&desired.filter_subject);
    current.filter_subjects.clear();
    current.ack_policy = desired.ack_policy;
    current.deliver_policy = desired.deliver_policy;
    current.replay_policy = desired.replay_policy;
    current.ack_wait = desired.ack_wait;
    current.max_deliver = desired.max_deliver;
    current.max_ack_pending = desired.max_ack_pending;
    current
}

pub(super) fn follows_contract(current: &consumer::Config, desired: &pull::Config) -> bool {
    with_contract_fields(current.clone(), desired) == *current
}

fn refuses_in_place_update(kind: &ConsumerUpdateErrorKind) -> bool {
    matches!(
        kind,
        ConsumerUpdateErrorKind::JetStream(error) if error.error_code() == ErrorCode::CONSUMER_CREATE
    )
}

pub(super) fn fill_ratio(bytes: u64, max_bytes: i64) -> f64 {
    if max_bytes <= 0 {
        return 0.0;
    }
    bytes as f64 / max_bytes as f64
}
