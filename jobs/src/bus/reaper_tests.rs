use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_nats::jetstream::consumer;
use backend_contracts::pipeline::{COLLAB_DATA_STORE, INDEX_AUDIO_STREAM};
use backend_contracts::worker_contract::{AUDIO_LANE, WorkerLane, WorkerLaneSpec};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::advisory::{AdvisoryReaper, ReapedLane, WorkerLostReceiver};
use super::worker_consumers::ConsumerProvision;
use super::worker_tests::{
    LostLeaseBench, RankedOutcomes, leak, live_bus, quick_timing, scratch_lane, scratch_stream,
    wait_until,
};
use super::*;
use crate::handlers::WorkerLostLane;
use crate::queue::{JobError, JobResult};

const SETTLE: Duration = Duration::from_millis(200);

async fn eventually<F, Fut>(what: &str, mut check: F) -> anyhow::Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = anyhow::Result<bool>>,
{
    let started = Instant::now();
    while !check().await? {
        anyhow::ensure!(
            started.elapsed() < Duration::from_secs(20),
            "timed out waiting until {what}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}

async fn leaves_the_queue(bench: &LostLeaseBench, stream_seq: u64) -> anyhow::Result<()> {
    eventually("the lost task left the queue", || async {
        Ok(!bench.is_queued(stream_seq).await?)
    })
    .await
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn a_lost_task_whose_advisory_never_reached_jobs_is_reaped_by_the_sweep() -> anyhow::Result<()>
{
    let bus = live_bus("sweep-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let queue = scratch_stream("SWEEP", &suffix);
    let durable = format!("audio-sweep-{suffix}");
    let mut bench = LostLeaseBench::open(&bus, &queue, &durable).await?;
    let lost = bench
        .publish("advisory-missed-while-jobs-restarted")
        .await?;
    bench.exhaust_deliveries(lost).await?;

    let outcomes = Arc::new(RankedOutcomes::default());
    let reaper = AdvisoryReaper::for_lanes(
        bus.clone(),
        outcomes.clone(),
        vec![scratch_lane(
            WorkerLane::Audio,
            &queue,
            &durable,
            &bench.subject,
            Some(WorkerLostLane::AudioIndex),
            SETTLE,
        )],
        quick_timing(Duration::from_millis(300)),
    );
    let cancellation = CancellationToken::new();
    let reaping = tokio::spawn(reaper.run(cancellation.clone()));

    wait_until("the sweep applied worker_lost", || {
        outcomes.worker_lost_calls() == 1
    })
    .await?;
    leaves_the_queue(&bench, lost).await?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(outcomes.worker_lost_calls(), 1);

    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), reaping).await???;
    bus.jetstream.delete_stream(queue.name).await?;
    Ok(())
}

#[derive(Default)]
struct FlakyReceiver {
    failures_left: AtomicUsize,
    refuse: bool,
    calls: AtomicUsize,
    applied: AtomicUsize,
}

impl FlakyReceiver {
    fn failing(times: usize) -> Self {
        Self {
            failures_left: AtomicUsize::new(times),
            ..Self::default()
        }
    }

    fn refusing() -> Self {
        Self {
            refuse: true,
            ..Self::default()
        }
    }
}

impl WorkerLostReceiver for FlakyReceiver {
    async fn apply_worker_lost(
        &self,
        _lane: WorkerLostLane,
        _stream_seq: u64,
        _payload: &[u8],
    ) -> JobResult {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.refuse {
            return Err(JobError::permanent(anyhow::anyhow!("not a task")));
        }
        let failing = self
            .failures_left
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok();
        if failing {
            return Err(JobError::retryable(anyhow::anyhow!("database restarts")));
        }
        self.applied.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

async fn reap_one(receiver: Arc<FlakyReceiver>, prefix: &str) -> anyhow::Result<()> {
    let bus = live_bus("reap-retry-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let queue = scratch_stream(prefix, &suffix);
    let durable = format!("audio-retry-{suffix}");
    let mut bench = LostLeaseBench::open(&bus, &queue, &durable).await?;
    let reaper = AdvisoryReaper::for_lanes(
        bus.clone(),
        receiver.clone(),
        vec![scratch_lane(
            WorkerLane::Audio,
            &queue,
            &durable,
            &bench.subject,
            Some(WorkerLostLane::AudioIndex),
            SETTLE,
        )],
        quick_timing(Duration::from_millis(500)),
    );
    let cancellation = CancellationToken::new();
    let reaping = tokio::spawn(reaper.run(cancellation.clone()));

    let lost = bench.publish("lost-during-an-outage").await?;
    bench.exhaust_deliveries(lost).await?;
    leaves_the_queue(&bench, lost).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;

    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), reaping).await???;
    bus.jetstream.delete_stream(queue.name).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn a_reap_that_outlives_its_apply_attempts_is_retried_until_it_lands() -> anyhow::Result<()> {
    let receiver = Arc::new(FlakyReceiver::failing(4));

    reap_one(receiver.clone(), "RETRY").await?;

    assert_eq!(receiver.applied.load(Ordering::SeqCst), 1);
    assert_eq!(receiver.calls.load(Ordering::SeqCst), 5);
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn a_task_the_receiver_refuses_for_good_leaves_the_queue_once() -> anyhow::Result<()> {
    let receiver = Arc::new(FlakyReceiver::refusing());

    reap_one(receiver.clone(), "REFUSED").await?;

    assert_eq!(receiver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(receiver.applied.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn a_worker_consumer_that_drifts_or_vanishes_is_brought_back_while_jobs_runs()
-> anyhow::Result<()> {
    let bus = live_bus("consumer-upkeep-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let queue = scratch_stream("UPKEEP", &suffix);
    let contract = WorkerLaneSpec {
        stream: queue,
        durable: leak(format!("audio-upkeep-{suffix}")),
        filter_subject: leak(format!("test.upkeep.{suffix}.new")),
        ..AUDIO_LANE
    };
    let stream = bus.ensure_stream(pipeline_stream_config(&queue)).await?;
    bus.ensure_worker_consumer(&contract).await?;
    let provisioned = stream.consumer_info(contract.durable).await?.config;
    stream
        .update_consumer(consumer::Config {
            max_ack_pending: 7,
            ..provisioned
        })
        .await?;

    let reaper = AdvisoryReaper::for_lanes(
        bus.clone(),
        Arc::new(RankedOutcomes::default()),
        vec![ReapedLane {
            consumer_contract: Some(contract),
            ..scratch_lane(
                WorkerLane::Audio,
                &queue,
                contract.durable,
                contract.filter_subject,
                None,
                SETTLE,
            )
        }],
        quick_timing(Duration::from_millis(300)),
    );
    let cancellation = CancellationToken::new();
    let reaping = tokio::spawn(reaper.run(cancellation.clone()));

    eventually(
        "the drifted consumer follows the contract again",
        || async {
            Ok(stream
                .consumer_info(contract.durable)
                .await?
                .config
                .max_ack_pending
                == 256)
        },
    )
    .await?;
    stream.delete_consumer(contract.durable).await?;
    eventually("the deleted consumer exists again", || async {
        Ok(stream.consumer_info(contract.durable).await.is_ok())
    })
    .await?;

    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), reaping).await???;
    bus.jetstream.delete_stream(queue.name).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn a_result_consumer_deleted_under_jobs_is_recreated_and_keeps_consuming()
-> anyhow::Result<()> {
    let bus = live_bus("result-consumer-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let queue = scratch_stream("RESULTS", &suffix);
    let subject = format!("test.results.{suffix}.done");
    let durable = format!("results-{suffix}");
    let stream = bus.ensure_stream(pipeline_stream_config(&queue)).await?;
    let results = bus
        .consumer(
            stream.clone(),
            &durable,
            &subject,
            &format!("test.dead.{suffix}"),
            1,
            Duration::from_millis(100),
        )
        .await?;
    let handled = Arc::new(AtomicUsize::new(0));
    let counter = handled.clone();
    let cancellation = CancellationToken::new();
    let consuming = tokio::spawn(results.run_raw(
        cancellation.clone(),
        move |_: serde_json::Value| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        },
    ));
    let publish = |label: &'static str| {
        let bus = bus.clone();
        let subject = subject.clone();
        async move { bus.publish(&subject, &json!({ "result": label })).await }
    };

    publish("before").await?;
    wait_until("the first result was handled", || {
        handled.load(Ordering::SeqCst) == 1
    })
    .await?;
    stream.delete_consumer(&durable).await?;
    publish("after").await?;
    wait_until("a result after the consumer vanished was handled", || {
        handled.load(Ordering::SeqCst) == 2
    })
    .await?;

    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), consuming).await???;
    bus.jetstream.delete_stream(queue.name).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn statuses_keep_arriving_while_the_queue_snapshot_hangs() -> anyhow::Result<()> {
    let bus = live_bus("status-snapshot-test").await?;
    let node = format!("gpu-test-{}", Uuid::now_v7().simple());
    let board = crate::health::worker_status::WorkerStatusBoard::new();
    board.require_lanes(&[WorkerLane::Encode]);
    let cancellation = CancellationToken::new();
    let listening = tokio::spawn(board.clone().run_with(
        bus.clone(),
        cancellation.clone(),
        std::future::pending::<WorkerQueueSnapshot>,
    ));
    let status = json!({
        "trust": "trusted",
        "uptime_s": 1.0,
        "lanes": { "encode": { "state": "serving" } },
    });

    let started = Instant::now();
    while !board.health(Instant::now()).is_healthy() {
        anyhow::ensure!(
            started.elapsed() < Duration::from_secs(10),
            "a hanging queue snapshot kept statuses from reaching the board"
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

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn a_worker_stream_deleted_under_jobs_is_recreated_by_the_consumer_check()
-> anyhow::Result<()> {
    let bus = live_bus("worker-stream-revival-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let queue = scratch_stream("REVIVAL", &suffix);
    let contract = WorkerLaneSpec {
        stream: queue,
        durable: leak(format!("audio-revival-{suffix}")),
        filter_subject: leak(format!("test.revival.{suffix}.new")),
        ..AUDIO_LANE
    };
    bus.ensure_stream(pipeline_stream_config(&queue)).await?;
    bus.ensure_worker_consumer(&contract).await?;

    bus.jetstream.delete_stream(queue.name).await?;

    assert_eq!(
        bus.ensure_worker_consumer(&contract).await?,
        ConsumerProvision::Created
    );
    let stream = bus.jetstream.get_stream(queue.name).await?;
    validate_stream(
        stream.cached_info().config.clone(),
        &pipeline_stream_config(&queue),
    )?;
    stream.consumer_info(contract.durable).await?;
    bus.jetstream.delete_stream(queue.name).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn a_result_stream_deleted_under_jobs_is_recreated_and_keeps_consuming() -> anyhow::Result<()>
{
    let bus = live_bus("result-stream-revival-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let queue = scratch_stream("RESULT_REVIVAL", &suffix);
    let subject = format!("test.result_revival.{suffix}.done");
    let durable = format!("result-revival-{suffix}");
    let stream = bus.ensure_stream(pipeline_stream_config(&queue)).await?;
    let results = bus
        .consumer(
            stream,
            &durable,
            &subject,
            &format!("test.dead.{suffix}"),
            1,
            Duration::from_millis(100),
        )
        .await?;
    let handled = Arc::new(AtomicUsize::new(0));
    let counter = handled.clone();
    let cancellation = CancellationToken::new();
    let consuming = tokio::spawn(results.run_raw(
        cancellation.clone(),
        move |_: serde_json::Value| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        },
    ));

    bus.publish(&subject, &json!({ "result": "before" }))
        .await?;
    wait_until("the first result was handled", || {
        handled.load(Ordering::SeqCst) == 1
    })
    .await?;
    bus.jetstream.delete_stream(queue.name).await?;
    eventually("the result stream accepts results again", || async {
        Ok(bus
            .publish(&subject, &json!({ "result": "after" }))
            .await
            .is_ok())
    })
    .await?;
    wait_until("a result after the stream vanished was handled", || {
        handled.load(Ordering::SeqCst) == 2
    })
    .await?;

    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), consuming).await???;
    bus.jetstream.delete_stream(queue.name).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a disposable JetStream server: it provisions the real stream names"]
async fn the_pipeline_topology_comes_back_while_jobs_runs_and_readiness_fails_while_it_cannot()
-> anyhow::Result<()> {
    let bus = live_bus("topology-test").await?;
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

    bus.jetstream.delete_stream(INDEX_AUDIO_STREAM.name).await?;
    bus.jetstream
        .delete_object_store(COLLAB_DATA_STORE.bucket)
        .await?;
    bus.restore_topology().await;

    bus.jetstream.get_stream(INDEX_AUDIO_STREAM.name).await?;
    bus.jetstream
        .get_object_store(COLLAB_DATA_STORE.bucket)
        .await?;
    assert!(bus.is_available().await);

    bus.jetstream.delete_stream(INDEX_AUDIO_STREAM.name).await?;
    let squatter = format!("SQUATTER_{}", Uuid::now_v7().simple());
    bus.jetstream
        .create_stream(StreamConfig {
            name: squatter.clone(),
            subjects: INDEX_AUDIO_STREAM
                .subjects
                .iter()
                .map(|subject| (*subject).to_owned())
                .collect(),
            ..Default::default()
        })
        .await?;
    bus.restore_topology().await;
    assert!(!bus.is_available().await);

    bus.jetstream.delete_stream(&squatter).await?;
    bus.restore_topology().await;
    assert!(bus.is_available().await);
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn a_lost_task_is_not_reaped_from_under_a_consumer_recreated_while_it_settled()
-> anyhow::Result<()> {
    let bus = live_bus("recreated-consumer-reap-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let queue = scratch_stream("RECREATED", &suffix);
    let durable = format!("audio-recreated-{suffix}");
    let mut bench = LostLeaseBench::open(&bus, &queue, &durable).await?;
    let settle = Duration::from_secs(2);
    let outcomes = Arc::new(RankedOutcomes::default());
    let reaper = AdvisoryReaper::for_lanes(
        bus.clone(),
        outcomes.clone(),
        vec![scratch_lane(
            WorkerLane::Audio,
            &queue,
            &durable,
            &bench.subject,
            Some(WorkerLostLane::AudioIndex),
            settle,
        )],
        quick_timing(Duration::from_secs(3600)),
    );
    let cancellation = CancellationToken::new();
    let reaping = tokio::spawn(reaper.run(cancellation.clone()));

    let lost = bench
        .publish("lost-then-redelivered-by-a-new-consumer")
        .await?;
    bench.exhaust_deliveries(lost).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    bench.stream.delete_consumer(&durable).await?;
    bench
        .stream
        .create_consumer(consumer::pull::Config {
            durable_name: Some(durable.clone()),
            filter_subject: bench.subject.clone(),
            ack_policy: AckPolicy::Explicit,
            ack_wait: Duration::from_secs(1),
            max_deliver: 2,
            ..Default::default()
        })
        .await?;
    tokio::time::sleep(settle + Duration::from_millis(1500)).await;

    assert_eq!(outcomes.worker_lost_calls(), 0);
    assert!(bench.is_queued(lost).await?);

    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), reaping).await???;
    bus.jetstream.delete_stream(queue.name).await?;
    Ok(())
}

struct ForgettingReceiver {
    stream: async_nats::jetstream::stream::Stream,
    failures: usize,
    calls: AtomicUsize,
    applied: AtomicUsize,
}

impl WorkerLostReceiver for ForgettingReceiver {
    async fn apply_worker_lost(
        &self,
        _lane: WorkerLostLane,
        stream_seq: u64,
        payload: &[u8],
    ) -> JobResult {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call == 1 {
            self.stream
                .delete_message(stream_seq)
                .await
                .map_err(|error| JobError::retryable(anyhow::Error::from(error)))?;
        }
        if call <= self.failures {
            return Err(JobError::retryable(anyhow::anyhow!("database restarts")));
        }
        assert!(payload.starts_with(b"{\"task\""));
        self.applied.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn a_confirmed_lost_task_is_applied_even_after_its_stream_forgot_it() -> anyhow::Result<()> {
    let bus = live_bus("forgotten-lost-task-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let queue = scratch_stream("FORGOTTEN", &suffix);
    let durable = format!("audio-forgotten-{suffix}");
    let mut bench = LostLeaseBench::open(&bus, &queue, &durable).await?;
    let receiver = Arc::new(ForgettingReceiver {
        stream: bench.stream.clone(),
        failures: 3,
        calls: AtomicUsize::new(0),
        applied: AtomicUsize::new(0),
    });
    let reaper = AdvisoryReaper::for_lanes(
        bus.clone(),
        receiver.clone(),
        vec![scratch_lane(
            WorkerLane::Audio,
            &queue,
            &durable,
            &bench.subject,
            Some(WorkerLostLane::AudioIndex),
            SETTLE,
        )],
        quick_timing(Duration::from_secs(3600)),
    );
    let cancellation = CancellationToken::new();
    let reaping = tokio::spawn(reaper.run(cancellation.clone()));

    let lost = bench.publish("expires-while-the-database-is-down").await?;
    bench.exhaust_deliveries(lost).await?;
    wait_until(
        "the saved task was applied after it left the stream",
        || receiver.applied.load(Ordering::SeqCst) == 1,
    )
    .await?;

    assert_eq!(receiver.calls.load(Ordering::SeqCst), 4);
    assert!(!bench.is_queued(lost).await?);
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), reaping).await???;
    bus.jetstream.delete_stream(queue.name).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn results_waiting_behind_a_slow_one_are_not_redelivered_behind_its_back()
-> anyhow::Result<()> {
    let bus = live_bus("waiting-results-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let queue = scratch_stream("WAITING", &suffix);
    let subject = format!("test.waiting.{suffix}.done");
    let durable = format!("waiting-{suffix}");
    let stream = bus.ensure_stream(pipeline_stream_config(&queue)).await?;
    let results = WorkConsumer::open(
        bus.jetstream.clone(),
        stream,
        pull::Config {
            durable_name: Some(durable.clone()),
            name: Some(durable.clone()),
            ack_policy: AckPolicy::Explicit,
            ack_wait: Duration::from_secs(2),
            max_deliver: -1,
            filter_subject: subject.clone(),
            max_ack_pending: 4,
            ..Default::default()
        },
        format!("test.dead.{suffix}"),
        1,
        Duration::from_millis(100),
    )
    .await?;
    for label in ["first", "second", "third"] {
        bus.publish(&subject, &json!({ "result": label })).await?;
    }
    let deliveries = Arc::new(Mutex::new(Vec::new()));
    let seen = deliveries.clone();
    let cancellation = CancellationToken::new();
    let consuming = tokio::spawn(results.run_raw_with_context(
        cancellation.clone(),
        move |_: serde_json::Value, context: DeliveryContext| {
            let seen = seen.clone();
            async move {
                seen.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push((context.stream_sequence, context.delivery_attempt));
                tokio::time::sleep(Duration::from_secs(3)).await;
                Ok(())
            }
        },
    ));
    let recorded = || {
        deliveries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    };

    wait_until("every result was handled", || recorded().len() >= 3).await?;
    tokio::time::sleep(Duration::from_secs(4)).await;

    let recorded = recorded();
    assert_eq!(recorded.len(), 3, "{recorded:?}");
    assert!(
        recorded.iter().all(|(_, attempt)| *attempt == 1),
        "{recorded:?}"
    );
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(10), consuming).await???;
    bus.jetstream.delete_stream(queue.name).await?;
    Ok(())
}
