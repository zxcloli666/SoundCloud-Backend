use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use async_nats::jetstream::ErrorCode;
use async_nats::jetstream::consumer;
use async_nats::jetstream::context::ConsumerInfoErrorKind;
use async_nats::jetstream::message::StreamMessage;
use async_nats::jetstream::stream::{DeleteMessageErrorKind, RawMessageErrorKind, Stream};
use backend_contracts::worker_contract::{WORKER_LANES, WorkerLane, WorkerLaneSpec};
use futures::StreamExt;
use serde::Deserialize;
use tokio::task::{Id, JoinError, JoinSet};
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use super::Bus;
use super::worker_consumers::ConsumerProvision;
use crate::handlers::{JobHandlers, WorkerLostLane};
use crate::metrics::WorkerLostOutcome;
use crate::queue::JobResult;

pub const MAX_DELIVERIES_ADVISORY: &str = "$JS.EVENT.ADVISORY.CONSUMER.MAX_DELIVERIES.*.*";
const SETTLE_MARGIN: Duration = Duration::from_secs(30);
const APPLY_ATTEMPTS: u32 = 3;
const APPLY_RETRY_DELAY: Duration = Duration::from_secs(30);
const MAX_REAP_BACKOFF: Duration = Duration::from_secs(10 * 60);
const MAX_CONCURRENT_REAPS: usize = 64;
const UPKEEP_INTERVAL: Duration = Duration::from_secs(60);
const UPKEEP_STEP_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_ORPHANS_PER_LANE: usize = 64;

pub trait WorkerLostReceiver: Send + Sync + 'static {
    fn apply_worker_lost(
        &self,
        lane: WorkerLostLane,
        stream_seq: u64,
        payload: &[u8],
    ) -> impl Future<Output = JobResult> + Send;
}

impl WorkerLostReceiver for JobHandlers {
    async fn apply_worker_lost(
        &self,
        lane: WorkerLostLane,
        stream_seq: u64,
        payload: &[u8],
    ) -> JobResult {
        JobHandlers::apply_worker_lost(self, lane, stream_seq, payload).await
    }
}

#[derive(Clone, Debug)]
pub(super) struct ReapedLane {
    pub(super) lane: WorkerLane,
    pub(super) stream: String,
    pub(super) durable: String,
    pub(super) filter_subject: String,
    pub(super) receiver: Option<WorkerLostLane>,
    pub(super) settle: Duration,
    pub(super) give_up_after: Duration,
    pub(super) consumer_contract: Option<WorkerLaneSpec>,
}

impl ReapedLane {
    fn from_contract(spec: &WorkerLaneSpec) -> Self {
        Self {
            lane: spec.lane,
            stream: spec.stream.name.to_owned(),
            durable: spec.durable.to_owned(),
            filter_subject: spec.filter_subject.to_owned(),
            receiver: receiver_of(spec.lane),
            settle: Duration::from_secs(spec.ack_wait_s) + SETTLE_MARGIN,
            give_up_after: Duration::from_secs(spec.stream.max_age_seconds),
            consumer_contract: Some(*spec),
        }
    }
}

fn receiver_of(lane: WorkerLane) -> Option<WorkerLostLane> {
    match lane {
        WorkerLane::Audio => Some(WorkerLostLane::AudioIndex),
        WorkerLane::Transcribe => Some(WorkerLostLane::Transcription),
        WorkerLane::Lyrics => Some(WorkerLostLane::LyricsEmbedding),
        WorkerLane::Encode | WorkerLane::Collab | WorkerLane::Taste | WorkerLane::Ai => None,
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ReaperTiming {
    pub(super) retry_delay: Duration,
    pub(super) upkeep_every: Duration,
}

#[derive(Debug, Deserialize)]
pub(super) struct MaxDeliveriesAdvisory {
    pub(super) stream: String,
    pub(super) consumer: String,
    pub(super) stream_seq: u64,
    #[serde(default)]
    pub(super) deliveries: u64,
}

pub struct AdvisoryReaper<R> {
    bus: Bus,
    receiver: Arc<R>,
    lanes: Vec<ReapedLane>,
    timing: ReaperTiming,
    restores_topology: bool,
}

#[derive(Default)]
struct Reaping {
    confirmed_payload: Option<Vec<u8>>,
    forgotten_by_stream_at: Option<Instant>,
}

impl<R: WorkerLostReceiver> AdvisoryReaper<R> {
    pub fn new(bus: Bus, receiver: Arc<R>) -> Self {
        Self {
            restores_topology: true,
            ..Self::for_lanes(
                bus,
                receiver,
                WORKER_LANES.iter().map(ReapedLane::from_contract).collect(),
                ReaperTiming {
                    retry_delay: APPLY_RETRY_DELAY,
                    upkeep_every: UPKEEP_INTERVAL,
                },
            )
        }
    }

    pub(super) fn for_lanes(
        bus: Bus,
        receiver: Arc<R>,
        lanes: Vec<ReapedLane>,
        timing: ReaperTiming,
    ) -> Self {
        Self {
            bus,
            receiver,
            lanes,
            timing,
            restores_topology: false,
        }
    }

    pub async fn run(self, cancellation: CancellationToken) -> anyhow::Result<()> {
        let mut advisories = self.bus.subscribe(MAX_DELIVERIES_ADVISORY).await?;
        let mut upkeep = tokio::time::interval(self.timing.upkeep_every);
        upkeep.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let reaper = Arc::new(self);
        let mut reaps = Reaps::default();

        let result = loop {
            let lost = tokio::select! {
                biased;
                _ = cancellation.cancelled() => break Ok(()),
                Some(finished) = reaps.tasks.join_next_with_id(), if !reaps.tasks.is_empty() => {
                    reaps.finish(finished);
                    continue;
                }
                message = advisories.next() => {
                    let Some(message) = message else {
                        break Err(anyhow::anyhow!("NATS max deliveries advisory subscription closed"));
                    };
                    reaper.claim(&message.payload).into_iter().collect::<Vec<_>>()
                }
                _ = upkeep.tick() => match cancellation.run_until_cancelled(reaper.upkeep()).await {
                    Some(orphans) => orphans,
                    None => break Ok(()),
                },
            };
            if !reaps.start(&reaper, lost, &cancellation).await {
                break Ok(());
            }
        };
        reaps.tasks.shutdown().await;
        result
    }

    fn claim(&self, payload: &[u8]) -> Option<(ReapedLane, u64)> {
        let advisory = match serde_json::from_slice::<MaxDeliveriesAdvisory>(payload) {
            Ok(advisory) => advisory,
            Err(error) => {
                tracing::warn!(%error, "NATS max deliveries advisory is not readable");
                return None;
            }
        };
        let lane = self
            .lanes
            .iter()
            .find(|lane| lane.durable == advisory.consumer && lane.stream == advisory.stream)?
            .clone();
        tracing::warn!(
            lane = lane.lane.as_str(),
            stream = %advisory.stream,
            durable = %advisory.consumer,
            stream_seq = advisory.stream_seq,
            deliveries = advisory.deliveries,
            "worker task ran out of deliveries"
        );
        Some((lane, advisory.stream_seq))
    }

    async fn upkeep(&self) -> Vec<(ReapedLane, u64)> {
        if self.restores_topology
            && tokio::time::timeout(UPKEEP_STEP_TIMEOUT, self.bus.restore_topology())
                .await
                .is_err()
        {
            tracing::warn!("NATS topology check timed out");
        }
        let mut orphans = Vec::new();
        for lane in &self.lanes {
            if let Some(contract) = &lane.consumer_contract {
                self.reconcile(contract).await;
            }
            match tokio::time::timeout(UPKEEP_STEP_TIMEOUT, self.orphans_of(lane)).await {
                Ok(Ok(found)) => orphans.extend(found.into_iter().map(|seq| (lane.clone(), seq))),
                Ok(Err(error)) => tracing::warn!(
                    lane = lane.lane.as_str(),
                    error = format!("{error:#}"),
                    "worker lane could not be swept for lost tasks"
                ),
                Err(_) => tracing::warn!(
                    lane = lane.lane.as_str(),
                    "worker lane sweep for lost tasks timed out"
                ),
            }
        }
        orphans
    }

    async fn reconcile(&self, contract: &WorkerLaneSpec) {
        let checked = tokio::time::timeout(
            UPKEEP_STEP_TIMEOUT,
            self.bus.ensure_worker_consumer(contract),
        )
        .await;
        match checked {
            Ok(Ok(ConsumerProvision::Unchanged)) => {}
            Ok(Ok(provision)) => tracing::warn!(
                lane = contract.lane.as_str(),
                durable = contract.durable,
                ?provision,
                "worker consumer had left the contract and was brought back"
            ),
            Ok(Err(error)) => tracing::warn!(
                lane = contract.lane.as_str(),
                durable = contract.durable,
                error = format!("{error:#}"),
                "worker consumer could not be checked against the contract"
            ),
            Err(_) => tracing::warn!(
                lane = contract.lane.as_str(),
                durable = contract.durable,
                "worker consumer check against the contract timed out"
            ),
        }
    }

    async fn orphans_of(&self, lane: &ReapedLane) -> anyhow::Result<Vec<u64>> {
        let stream = self
            .bus
            .jetstream
            .get_stream(&lane.stream)
            .await
            .with_context(|| format!("NATS stream {} could not be loaded", lane.stream))?;
        let info = match stream.consumer_info(&lane.durable).await {
            Ok(info) => info,
            Err(error) if error.kind() == ConsumerInfoErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("NATS worker consumer {} could not be read", lane.durable)
                });
            }
        };
        let horizon = delivery_horizon(&info);
        let mut cursor = stream.cached_info().state.first_sequence.max(1);
        let mut orphans = Vec::new();
        while cursor <= horizon && orphans.len() < MAX_ORPHANS_PER_LANE {
            let Some(stream_seq) = next_queued(&stream, cursor, &lane.filter_subject).await? else {
                break;
            };
            if stream_seq > horizon {
                break;
            }
            orphans.push(stream_seq);
            cursor = stream_seq.saturating_add(1);
        }
        if !orphans.is_empty() {
            tracing::info!(
                lane = lane.lane.as_str(),
                durable = %lane.durable,
                count = orphans.len(),
                "worker tasks that will never be delivered again are still queued"
            );
        }
        Ok(orphans)
    }

    async fn reap(&self, lane: ReapedLane, stream_seq: u64) {
        let started = Instant::now();
        let mut reaping = Reaping::default();
        let mut backoff = self.timing.retry_delay;
        let outcome = loop {
            match self.settle(&lane, stream_seq, &mut reaping).await {
                Ok(outcome) => break outcome,
                Err(error)
                    if Instant::now() + backoff
                        < reaping
                            .forgotten_by_stream_at
                            .unwrap_or(started + lane.give_up_after) =>
                {
                    tracing::warn!(
                        lane = lane.lane.as_str(),
                        stream_seq,
                        retry_in_s = backoff.as_secs_f64(),
                        error = format!("{error:#}"),
                        "worker lost task could not be settled yet"
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = backoff.saturating_mul(2).min(MAX_REAP_BACKOFF);
                }
                Err(error) => {
                    tracing::error!(
                        lane = lane.lane.as_str(),
                        stream_seq,
                        error = format!("{error:#}"),
                        "worker lost task could not be settled before its stream forgets it"
                    );
                    break WorkerLostOutcome::Failed;
                }
            }
        };
        tracing::info!(
            lane = lane.lane.as_str(),
            stream_seq,
            outcome = outcome.as_str(),
            "worker lost task settled"
        );
        crate::metrics::record_worker_lost(lane.lane, outcome);
    }

    async fn settle(
        &self,
        lane: &ReapedLane,
        stream_seq: u64,
        reaping: &mut Reaping,
    ) -> anyhow::Result<WorkerLostOutcome> {
        let stream = self
            .bus
            .jetstream
            .get_stream(&lane.stream)
            .await
            .with_context(|| format!("NATS stream {} could not be loaded", lane.stream))?;
        let payload = match &reaping.confirmed_payload {
            Some(payload) => payload.clone(),
            None => {
                let Some(payload) = confirm_lost(&stream, lane, stream_seq, reaping).await? else {
                    return Ok(WorkerLostOutcome::Settled);
                };
                reaping.confirmed_payload.insert(payload).clone()
            }
        };

        let outcome = match lane.receiver {
            Some(receiver) => self.apply(receiver, stream_seq, &payload).await?,
            None => WorkerLostOutcome::Dropped,
        };
        forget(&stream, stream_seq).await?;
        Ok(outcome)
    }

    async fn apply(
        &self,
        receiver: WorkerLostLane,
        stream_seq: u64,
        payload: &[u8],
    ) -> anyhow::Result<WorkerLostOutcome> {
        let mut attempt = 1;
        loop {
            match self
                .receiver
                .apply_worker_lost(receiver, stream_seq, payload)
                .await
            {
                Ok(()) => return Ok(WorkerLostOutcome::Applied),
                Err(error) if error.is_retryable() && attempt < APPLY_ATTEMPTS => {
                    tracing::warn!(
                        ?receiver,
                        stream_seq,
                        attempt,
                        %error,
                        "worker lost receiver asked to retry"
                    );
                    attempt += 1;
                    tokio::time::sleep(self.timing.retry_delay).await;
                }
                Err(error) if error.is_retryable() => {
                    return Err(anyhow::anyhow!(
                        "worker lost receiver could not take seq {stream_seq}: {error}"
                    ));
                }
                Err(error) => {
                    tracing::error!(
                        ?receiver,
                        stream_seq,
                        %error,
                        "worker lost receiver refused the task for good"
                    );
                    return Ok(WorkerLostOutcome::Failed);
                }
            }
        }
    }
}

#[derive(Default)]
pub(super) struct Reaps {
    tasks: JoinSet<()>,
    in_flight: HashMap<Id, (String, u64)>,
}

impl Reaps {
    async fn start<R: WorkerLostReceiver>(
        &mut self,
        reaper: &Arc<AdvisoryReaper<R>>,
        lost: Vec<(ReapedLane, u64)>,
        cancellation: &CancellationToken,
    ) -> bool {
        for (lane, stream_seq) in lost {
            let key = (lane.stream.clone(), stream_seq);
            if self.in_flight.values().any(|running| *running == key) {
                continue;
            }
            if !self.admit(cancellation).await {
                return false;
            }
            let reaper = reaper.clone();
            let handle = self
                .tasks
                .spawn(async move { reaper.reap(lane, stream_seq).await });
            self.in_flight.insert(handle.id(), key);
        }
        true
    }

    pub(super) async fn admit(&mut self, cancellation: &CancellationToken) -> bool {
        while self.tasks.len() >= MAX_CONCURRENT_REAPS {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => return false,
                finished = self.tasks.join_next_with_id() => match finished {
                    Some(finished) => self.finish(finished),
                    None => break,
                },
            }
        }
        true
    }

    fn finish(&mut self, finished: Result<(Id, ()), JoinError>) {
        let id = match finished {
            Ok((id, ())) => id,
            Err(error) => {
                tracing::error!(%error, "worker lost reap task failed");
                error.id()
            }
        };
        self.in_flight.remove(&id);
    }
}

async fn next_queued(stream: &Stream, from: u64, subject: &str) -> anyhow::Result<Option<u64>> {
    match stream
        .raw_message_builder()
        .sequence(from)
        .next_by_subject(subject.to_owned())
        .send()
        .await
    {
        Ok(message) => Ok(Some(message.sequence)),
        Err(error) if error.kind() == RawMessageErrorKind::NoMessageFound => Ok(None),
        Err(error) => Err(error).context("NATS worker queue could not be walked"),
    }
}

async fn confirm_lost(
    stream: &Stream,
    lane: &ReapedLane,
    stream_seq: u64,
    reaping: &mut Reaping,
) -> anyhow::Result<Option<Vec<u8>>> {
    let Some(message) = still_queued(stream, stream_seq).await? else {
        return Ok(None);
    };
    reaping.forgotten_by_stream_at = Some(
        Instant::now()
            + remaining_life(
                message.time.unix_timestamp(),
                unix_now(),
                lane.give_up_after,
            ),
    );
    let owner = consumer_generation(stream, &lane.durable).await?;
    tokio::time::sleep(lane.settle).await;
    if still_queued(stream, stream_seq).await?.is_none() {
        return Ok(None);
    }
    if owner.is_none() || consumer_generation(stream, &lane.durable).await? != owner {
        tracing::info!(
            lane = lane.lane.as_str(),
            durable = %lane.durable,
            stream_seq,
            "worker consumer was replaced while a lost task settled, the new one delivers it again"
        );
        return Ok(None);
    }
    Ok(Some(message.payload.to_vec()))
}

fn delivery_horizon(info: &consumer::Info) -> u64 {
    if info.num_ack_pending == 0 {
        info.delivered.stream_sequence
    } else {
        info.ack_floor.stream_sequence
    }
}

async fn consumer_generation(stream: &Stream, durable: &str) -> anyhow::Result<Option<i128>> {
    match stream.consumer_info(durable).await {
        Ok(info) => Ok(Some(info.created.unix_timestamp_nanos())),
        Err(error) if error.kind() == ConsumerInfoErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("NATS worker consumer {durable} could not be read"))
        }
    }
}

fn remaining_life(published_unix_s: i64, now_unix_s: i64, max_age: Duration) -> Duration {
    let age = u64::try_from(now_unix_s.saturating_sub(published_unix_s)).unwrap_or(0);
    max_age.saturating_sub(Duration::from_secs(age))
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_secs()).ok())
        .unwrap_or(i64::MAX)
}

async fn still_queued(stream: &Stream, stream_seq: u64) -> anyhow::Result<Option<StreamMessage>> {
    match stream.get_raw_message(stream_seq).await {
        Ok(message) => Ok(Some(message)),
        Err(error) if error.kind() == RawMessageErrorKind::NoMessageFound => Ok(None),
        Err(error) => Err(error).context("NATS worker task could not be read"),
    }
}

async fn forget(stream: &Stream, stream_seq: u64) -> anyhow::Result<()> {
    match stream.delete_message(stream_seq).await {
        Ok(_) => Ok(()),
        Err(error) if is_already_gone(&error.kind()) => Ok(()),
        Err(error) => {
            Err(error).context("NATS worker task could not be removed after it was settled")
        }
    }
}

fn is_already_gone(kind: &DeleteMessageErrorKind) -> bool {
    matches!(
        kind,
        DeleteMessageErrorKind::JetStream(error) if error.error_code() == ErrorCode::NO_MESSAGE_FOUND
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_lane_the_contract_reopens_on_worker_lost_has_a_receiver() {
        for spec in &WORKER_LANES {
            let promises_reopen = spec
                .lane
                .reopenable()
                .contains(&backend_contracts::reasons::WorkerReason::WorkerLost);
            let stateless_retrain = matches!(spec.lane, WorkerLane::Collab | WorkerLane::Taste);
            assert!(
                !promises_reopen || stateless_retrain || receiver_of(spec.lane).is_some(),
                "lane {} promises a worker_lost reopen that jobs never applies",
                spec.lane.as_str()
            );
        }
        assert_eq!(
            receiver_of(WorkerLane::Lyrics),
            Some(WorkerLostLane::LyricsEmbedding)
        );
    }

    #[test]
    fn a_lane_gives_up_on_a_lost_task_only_when_its_stream_would_forget_it() {
        let lane = ReapedLane::from_contract(&backend_contracts::worker_contract::TRANSCRIBE_LANE);

        assert_eq!(lane.give_up_after, Duration::from_secs(24 * 60 * 60));
        assert_eq!(lane.settle, Duration::from_secs(150));
        assert!(lane.consumer_contract.is_some());
    }

    #[test]
    fn a_reap_gives_up_when_the_stream_forgets_the_task_not_a_full_age_after_the_reap_began() {
        let day = Duration::from_secs(24 * 60 * 60);
        let published = 1_700_000_000;

        assert_eq!(
            remaining_life(published, published + 20 * 60 * 60, day),
            Duration::from_secs(4 * 60 * 60)
        );
        assert_eq!(
            remaining_life(published, published + 25 * 60 * 60, day),
            Duration::ZERO
        );
        assert_eq!(remaining_life(published, published - 5, day), day);
    }

    #[tokio::test]
    async fn waiting_for_a_free_reap_slot_ends_as_soon_as_jobs_shuts_down() -> anyhow::Result<()> {
        let mut reaps = Reaps::default();
        for _ in 0..MAX_CONCURRENT_REAPS {
            reaps.tasks.spawn(std::future::pending::<()>());
        }
        let cancellation = CancellationToken::new();
        let shutdown = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
            cancellation.cancel();
        };

        let (admitted, ()) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(reaps.admit(&cancellation), shutdown)
        })
        .await?;

        assert!(!admitted);
        reaps.tasks.shutdown().await;
        Ok(())
    }
}
