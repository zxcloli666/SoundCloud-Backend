use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_nats::jetstream::consumer::{self, DeliverPolicy, PullConsumer, ReplayPolicy};
use async_nats::jetstream::stream::RawMessageErrorKind;
use backend_contracts::pipeline::WORKER_STREAMS;
use backend_contracts::reasons::{WorkerReason, WorkerStatus, outcome_rank};
use backend_contracts::worker_contract::{AUDIO_LANE, WORKER_LANES, WorkerLane, WorkerLaneSpec};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::advisory::{
    AdvisoryReaper, MAX_DELIVERIES_ADVISORY, MaxDeliveriesAdvisory, ReapedLane, ReaperTiming,
    WorkerLostReceiver,
};
use super::worker_consumers::ConsumerProvision;
use super::*;
use crate::handlers::WorkerLostLane;
use crate::queue::JobResult;

pub(super) fn leak(value: String) -> &'static str {
    Box::leak(value.into_boxed_str())
}

pub(super) async fn live_bus(name: &str) -> anyhow::Result<Bus> {
    let config = NatsConfig {
        url: std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned()),
        job_ingress_concurrency: 1,
        impression_concurrency: 1,
        retry_delay: Duration::from_millis(100),
        max_age: Duration::from_secs(60 * 60),
        job_stream_max_bytes: 16 * 1024 * 1024,
        impression_stream_max_bytes: 16 * 1024 * 1024,
    };
    Bus::connect(&config, name).await
}

pub(super) fn scratch_stream(prefix: &str, suffix: &str) -> PipelineStreamSpec {
    let subject = leak(format!("test.{}.{suffix}.>", prefix.to_lowercase()));
    PipelineStreamSpec {
        name: leak(format!("{prefix}_{suffix}")),
        subjects: Box::leak(vec![subject].into_boxed_slice()),
        work_queue: true,
        max_age_seconds: 60 * 60,
        duplicate_window_seconds: 60,
        max_bytes: 16 * 1024 * 1024,
        discard: StreamDiscard::New,
    }
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn worker_consumers_are_created_updated_and_recreated_to_the_contract() -> anyhow::Result<()>
{
    let bus = live_bus("worker-consumer-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let stream_spec = scratch_stream("WORKER_CONSUMER", &suffix);
    let spec = WorkerLaneSpec {
        stream: stream_spec,
        durable: leak(format!("audio-test-{suffix}")),
        filter_subject: leak(format!("test.worker_consumer.{suffix}.new")),
        ..AUDIO_LANE
    };
    let stream = bus
        .ensure_stream(pipeline_stream_config(&stream_spec))
        .await?;

    assert_eq!(
        bus.ensure_worker_consumer(&spec).await?,
        ConsumerProvision::Created
    );
    assert_eq!(
        bus.ensure_worker_consumer(&spec).await?,
        ConsumerProvision::Unchanged
    );
    let created = stream.consumer_info(spec.durable).await?.config;
    assert_eq!(created.max_deliver, 5);
    assert_eq!(created.ack_wait, Duration::from_secs(60));
    assert_eq!(created.max_ack_pending, 256);
    assert_eq!(created.filter_subject, spec.filter_subject);
    assert_eq!(created.deliver_policy, DeliverPolicy::All);
    assert_eq!(created.replay_policy, ReplayPolicy::Instant);
    assert_eq!(created.ack_policy, AckPolicy::Explicit);

    stream
        .update_consumer(consumer::Config {
            max_ack_pending: 7,
            ack_wait: Duration::from_secs(30),
            ..created.clone()
        })
        .await?;
    assert_eq!(
        bus.ensure_worker_consumer(&spec).await?,
        ConsumerProvision::Updated
    );
    let updated = stream.consumer_info(spec.durable).await?.config;
    assert_eq!(updated.max_ack_pending, 256);
    assert_eq!(updated.ack_wait, Duration::from_secs(60));

    bus.jetstream
        .publish(spec.filter_subject.to_owned(), "waiting".into())
        .await?
        .await?;
    stream.delete_consumer(spec.durable).await?;
    stream
        .create_consumer(consumer::pull::Config {
            durable_name: Some(spec.durable.to_owned()),
            filter_subject: spec.filter_subject.to_owned(),
            ack_policy: AckPolicy::Explicit,
            replay_policy: ReplayPolicy::Original,
            max_deliver: -1,
            ..Default::default()
        })
        .await?;
    assert_eq!(
        bus.ensure_worker_consumer(&spec).await?,
        ConsumerProvision::Recreated
    );
    let recreated = stream.consumer_info(spec.durable).await?;
    assert_eq!(recreated.config.replay_policy, ReplayPolicy::Instant);
    assert_eq!(recreated.config.max_deliver, 5);
    assert_eq!(
        recreated.num_pending, 1,
        "a task that waited in the work queue must reach the recreated consumer"
    );

    bus.jetstream.delete_stream(stream_spec.name).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a disposable JetStream server: it provisions the real stream names"]
async fn provision_builds_every_contract_stream_and_worker_consumer() -> anyhow::Result<()> {
    let bus = live_bus("provision-test").await?;
    let retired = StreamConfig {
        name: "TRAIN_QUALITY".to_owned(),
        subjects: vec!["train.quality.>".to_owned()],
        ..Default::default()
    };
    bus.jetstream.get_or_create_stream(retired).await?;
    let config = NatsConfig {
        url: String::new(),
        job_ingress_concurrency: 1,
        impression_concurrency: 1,
        retry_delay: Duration::from_millis(100),
        max_age: Duration::from_secs(60 * 60),
        job_stream_max_bytes: 16 * 1024 * 1024,
        impression_stream_max_bytes: 16 * 1024 * 1024,
    };

    bus.provision(&config).await?;
    bus.provision(&config).await?;

    assert!(bus.jetstream.get_stream("TRAIN_QUALITY").await.is_err());
    for spec in PIPELINE_STREAMS {
        let stream = bus.jetstream.get_stream(spec.name).await?;
        validate_stream(
            stream.cached_info().config.clone(),
            &pipeline_stream_config(spec),
        )?;
    }
    for spec in &WORKER_LANES {
        let stream = bus.jetstream.get_stream(spec.stream.name).await?;
        let current = stream.consumer_info(spec.durable).await?.config;
        assert!(
            worker_consumers::follows_contract(
                &current,
                &worker_consumers::worker_consumer_config(spec)?
            ),
            "{} does not follow the contract",
            spec.durable
        );
    }
    for store in &WORKER_OBJECT_STORES {
        bus.jetstream.get_object_store(store.bucket).await?;
    }
    let snapshot = bus.worker_queue_snapshot().await;
    assert_eq!(snapshot.consumers.len(), WORKER_LANES.len());
    assert_eq!(snapshot.streams.len(), WORKER_STREAMS.len());
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn a_retired_stream_is_removed_once_and_its_absence_is_not_an_error() -> anyhow::Result<()> {
    let bus = live_bus("retired-stream-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let retired = scratch_stream("RETIRED", &suffix);
    bus.ensure_stream(pipeline_stream_config(&retired)).await?;

    bus.remove_retired_streams(&[retired.name]).await?;
    bus.remove_retired_streams(&[retired.name]).await?;

    assert!(bus.jetstream.get_stream(retired.name).await.is_err());
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn a_request_carries_its_deadline_reply_inbox_and_message_id() -> anyhow::Result<()> {
    let bus = live_bus("request-headers-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let rpc = scratch_stream("RPC_HEADERS", &suffix);
    let subject = format!("test.rpc_headers.{suffix}.resolve");
    let stream = bus.ensure_stream(pipeline_stream_config(&rpc)).await?;
    let timeout = Duration::from_secs(5);

    let sent_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let caller = bus.clone();
    let request_subject = subject.clone();
    let request = tokio::spawn(async move {
        caller
            .request::<_, String>(
                &request_subject,
                &json!({ "title": "x" }),
                timeout,
                "rpc:42",
            )
            .await
    });

    let consumer: PullConsumer = stream
        .create_consumer(consumer::pull::Config {
            filter_subject: subject.clone(),
            ack_policy: AckPolicy::Explicit,
            ..Default::default()
        })
        .await?;
    let mut batch = consumer
        .fetch()
        .max_messages(1)
        .expires(Duration::from_secs(5))
        .messages()
        .await?;
    let message = batch
        .next()
        .await
        .ok_or_else(|| anyhow::anyhow!("the request never reached the stream"))?
        .map_err(|error| anyhow::anyhow!("fetch failed: {error}"))?;
    let headers = message
        .headers
        .clone()
        .ok_or_else(|| anyhow::anyhow!("the request has no headers"))?;
    let header = |name: &str| headers.get(name).map(|value| value.as_str().to_owned());

    assert_eq!(header(MSG_ID_HEADER).as_deref(), Some("rpc:42"));
    let deadline: u128 = header(DEADLINE_HEADER)
        .ok_or_else(|| anyhow::anyhow!("the request has no deadline"))?
        .parse()?;
    let received_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    assert!(deadline >= sent_at + timeout.as_millis());
    assert!(deadline <= received_at + timeout.as_millis());
    let reply_to =
        header(REPLY_TO_HEADER).ok_or_else(|| anyhow::anyhow!("the request has no inbox"))?;

    bus.client
        .publish(
            reply_to,
            json!({ "ok": true, "data": "pong" }).to_string().into(),
        )
        .await?;
    message
        .ack()
        .await
        .map_err(|error| anyhow::anyhow!("ack failed: {error}"))?;

    assert_eq!(request.await??.as_deref(), Some("pong"));
    bus.jetstream.delete_stream(rpc.name).await?;
    Ok(())
}

#[derive(Default)]
pub(super) struct RankedOutcomes {
    rows: Mutex<HashMap<u64, (u8, &'static str)>>,
    worker_lost_calls: AtomicUsize,
}

impl RankedOutcomes {
    fn write(&self, stream_seq: u64, status: WorkerStatus, reason: Option<WorkerReason>) {
        let rank = outcome_rank(status, reason);
        let label = reason.map_or(status.as_str(), WorkerReason::as_str);
        let mut rows = self
            .rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = rows.entry(stream_seq).or_insert((0, "none"));
        if entry.0 <= rank {
            *entry = (rank, label);
        }
    }

    fn row(&self, stream_seq: u64) -> Option<&'static str> {
        self.rows
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&stream_seq)
            .map(|(_, label)| *label)
    }

    pub(super) fn worker_lost_calls(&self) -> usize {
        self.worker_lost_calls.load(Ordering::SeqCst)
    }
}

impl WorkerLostReceiver for RankedOutcomes {
    async fn apply_worker_lost(
        &self,
        lane: WorkerLostLane,
        stream_seq: u64,
        payload: &[u8],
    ) -> JobResult {
        assert_eq!(lane, WorkerLostLane::AudioIndex);
        assert!(payload.starts_with(b"{\"task\""));
        self.write(
            stream_seq,
            WorkerStatus::Failed,
            Some(WorkerReason::WorkerLost),
        );
        self.worker_lost_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

pub(super) struct LostLeaseBench {
    pub(super) bus: Bus,
    pub(super) stream: async_nats::jetstream::stream::Stream,
    pub(super) consumer: PullConsumer,
    pub(super) subject: String,
    pub(super) advisories: Subscriber,
}

impl LostLeaseBench {
    pub(super) async fn open(
        bus: &Bus,
        queue: &PipelineStreamSpec,
        durable: &str,
    ) -> anyhow::Result<Self> {
        let subject = queue
            .subjects
            .first()
            .map(|wildcard| wildcard.replace('>', "new"))
            .context("scratch stream has no subject")?;
        let stream = bus.ensure_stream(pipeline_stream_config(queue)).await?;
        let consumer: PullConsumer = stream
            .create_consumer(consumer::pull::Config {
                durable_name: Some(durable.to_owned()),
                filter_subject: subject.clone(),
                ack_policy: AckPolicy::Explicit,
                ack_wait: Duration::from_secs(1),
                max_deliver: 2,
                ..Default::default()
            })
            .await?;
        let advisories = bus
            .client
            .subscribe(format!(
                "$JS.EVENT.ADVISORY.CONSUMER.MAX_DELIVERIES.{}.{durable}",
                queue.name
            ))
            .await?;
        Ok(Self {
            bus: bus.clone(),
            stream,
            consumer,
            subject,
            advisories,
        })
    }

    pub(super) async fn publish(&self, task: &str) -> anyhow::Result<u64> {
        let payload = json!({ "task": task }).to_string();
        Ok(self
            .bus
            .jetstream
            .publish(self.subject.clone(), payload.into())
            .await?
            .await?
            .sequence)
    }

    pub(super) async fn exhaust_deliveries(
        &mut self,
        stream_seq: u64,
    ) -> anyhow::Result<MaxDeliveriesAdvisory> {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(30) {
            let mut batch = self
                .consumer
                .fetch()
                .max_messages(1)
                .expires(Duration::from_millis(1500))
                .messages()
                .await?;
            while let Some(message) = batch.next().await {
                message.map_err(|error| anyhow::anyhow!("fetch failed: {error}"))?;
            }
            let advisory =
                tokio::time::timeout(Duration::from_millis(100), self.advisories.next()).await;
            if let Ok(Some(message)) = advisory {
                let advisory: MaxDeliveriesAdvisory = serde_json::from_slice(&message.payload)?;
                if advisory.stream_seq == stream_seq {
                    return Ok(advisory);
                }
            }
        }
        anyhow::bail!("no max deliveries advisory arrived for seq {stream_seq}")
    }

    pub(super) async fn is_queued(&self, stream_seq: u64) -> anyhow::Result<bool> {
        match self.stream.get_raw_message(stream_seq).await {
            Ok(_) => Ok(true),
            Err(error) if error.kind() == RawMessageErrorKind::NoMessageFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}

pub(super) fn scratch_lane(
    lane: WorkerLane,
    queue: &PipelineStreamSpec,
    durable: &str,
    filter_subject: &str,
    receiver: Option<WorkerLostLane>,
    settle: Duration,
) -> ReapedLane {
    ReapedLane {
        lane,
        stream: queue.name.to_owned(),
        durable: durable.to_owned(),
        filter_subject: filter_subject.to_owned(),
        receiver,
        settle,
        give_up_after: Duration::from_secs(60),
        consumer_contract: None,
    }
}

pub(super) fn quick_timing(upkeep_every: Duration) -> ReaperTiming {
    ReaperTiming {
        retry_delay: Duration::from_millis(100),
        upkeep_every,
    }
}

pub(super) async fn wait_until(what: &str, check: impl Fn() -> bool) -> anyhow::Result<()> {
    let started = Instant::now();
    while !check() {
        anyhow::ensure!(
            started.elapsed() < Duration::from_secs(20),
            "timed out waiting until {what}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn a_task_that_ran_out_of_deliveries_is_reaped_and_a_late_ok_still_wins() -> anyhow::Result<()>
{
    let bus = live_bus("advisory-reaper-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let queue = scratch_stream("ADVISORY", &suffix);
    let subject = format!("test.advisory.{suffix}.new");
    let durable = format!("audio-advisory-{suffix}");
    let stream = bus.ensure_stream(pipeline_stream_config(&queue)).await?;
    let consumer: PullConsumer = stream
        .create_consumer(consumer::pull::Config {
            durable_name: Some(durable.clone()),
            filter_subject: subject.clone(),
            ack_policy: AckPolicy::Explicit,
            ack_wait: Duration::from_secs(1),
            max_deliver: 2,
            ..Default::default()
        })
        .await?;
    let advisories = bus
        .client
        .subscribe(format!(
            "$JS.EVENT.ADVISORY.CONSUMER.MAX_DELIVERIES.{}.{durable}",
            queue.name
        ))
        .await?;
    assert!(MAX_DELIVERIES_ADVISORY.ends_with(".*.*"));

    let outcomes = Arc::new(RankedOutcomes::default());
    let settle = Duration::from_millis(1500);
    let reaper = AdvisoryReaper::for_lanes(
        bus.clone(),
        outcomes.clone(),
        vec![
            scratch_lane(
                WorkerLane::Audio,
                &queue,
                &durable,
                &subject,
                Some(WorkerLostLane::AudioIndex),
                settle,
            ),
            scratch_lane(
                WorkerLane::Lyrics,
                &queue,
                &format!("lyrics-advisory-{suffix}"),
                &format!("test.advisory.{suffix}.lyrics"),
                None,
                settle,
            ),
        ],
        quick_timing(Duration::from_secs(3600)),
    );
    let cancellation = CancellationToken::new();
    let reaping = tokio::spawn(reaper.run(cancellation.clone()));
    let mut bench = LostLeaseBench {
        bus: bus.clone(),
        stream,
        consumer,
        subject,
        advisories,
    };

    let first = bench.publish("advisory-then-late-ok").await?;
    let advisory = bench.exhaust_deliveries(first).await?;
    assert_eq!(advisory.consumer, durable);
    assert_eq!(advisory.stream, queue.name);
    wait_until("the reaper applied worker_lost", || {
        outcomes.worker_lost_calls() == 1
    })
    .await?;
    assert_eq!(outcomes.row(first), Some("worker_lost"));
    outcomes.write(first, WorkerStatus::Ok, None);
    assert_eq!(outcomes.row(first), Some("ok"));
    let started = Instant::now();
    while bench.is_queued(first).await? {
        anyhow::ensure!(
            started.elapsed() < Duration::from_secs(5),
            "the reaped task never left the stream"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let second = bench.publish("late-ok-before-reaper").await?;
    bench.exhaust_deliveries(second).await?;
    outcomes.write(second, WorkerStatus::Ok, None);
    wait_until("the reaper applied worker_lost", || {
        outcomes.worker_lost_calls() == 2
    })
    .await?;
    assert_eq!(
        outcomes.row(second),
        Some("ok"),
        "worker_lost must not overwrite an ok that arrived first"
    );

    let third = bench.publish("acknowledged-while-reaper-waits").await?;
    bench.exhaust_deliveries(third).await?;
    bench.stream.delete_message(third).await?;
    tokio::time::sleep(settle + Duration::from_secs(1)).await;
    assert_eq!(
        outcomes.worker_lost_calls(),
        2,
        "a task that left the queue before the reaper looked again is not reported lost"
    );
    assert_eq!(outcomes.row(third), None);

    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), reaping).await???;
    bus.jetstream.delete_stream(queue.name).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn a_status_published_by_a_worker_reaches_the_board() -> anyhow::Result<()> {
    let bus = live_bus("worker-status-test").await?;
    let node = format!("gpu-test-{}", Uuid::now_v7().simple());
    let board = crate::health::worker_status::WorkerStatusBoard::new();
    board.require_lanes(&[WorkerLane::Encode]);
    let cancellation = CancellationToken::new();
    let listening = tokio::spawn(board.clone().run(bus.clone(), cancellation.clone()));
    let status = json!({
        "worker_id": node,
        "trust": "trusted",
        "uptime_s": 1.0,
        "lanes": { "encode": { "state": "serving", "done": { "ok": 1 } } },
    });

    let started = Instant::now();
    while !board.health(Instant::now()).is_healthy() {
        anyhow::ensure!(
            started.elapsed() < Duration::from_secs(10),
            "the published status never reached the board"
        );
        bus.client
            .publish(format!("worker.status.{node}"), status.to_string().into())
            .await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), listening).await???;
    Ok(())
}
