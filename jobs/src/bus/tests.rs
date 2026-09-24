use std::sync::Arc;

use async_nats::HeaderMap;
use backend_contracts::{
    EmptyPayload, Impression, ImpressionBatch, JobCommand, JobKind, Versioned,
};
use chrono::Utc;
use sqlx::PgPool;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::*;
use crate::queue::JobError;

#[test]
fn jobs_owns_every_pipeline_stream_contract() {
    let streams = PIPELINE_STREAMS
        .iter()
        .map(pipeline_stream_config)
        .collect::<Vec<_>>();
    let stream = |name: &str| streams.iter().find(|stream| stream.name == name);

    assert_eq!(streams.len(), PIPELINE_STREAMS.len());
    assert!(
        streams
            .iter()
            .all(|stream| stream.storage == StorageType::File)
    );
    for stream_name in ["INDEX_AUDIO", "EMBED_LYRICS", "TRANSCRIBE"] {
        assert_eq!(
            streams
                .iter()
                .find(|stream| stream.name == stream_name)
                .map(|stream| stream.duplicate_window),
            Some(Duration::from_secs(24 * 60 * 60))
        );
    }
    assert_eq!(
        streams
            .iter()
            .find(|stream| stream.name == "ENCODE")
            .map(|stream| stream.duplicate_window),
        Some(Duration::from_secs(15 * 60))
    );
    assert_eq!(
        streams
            .iter()
            .find(|stream| stream.name == "PIPELINE_DONE")
            .map(|stream| stream.retention),
        Some(RetentionPolicy::Limits)
    );
    assert!(
        streams
            .iter()
            .all(|stream| stream.duplicate_window > Duration::ZERO)
    );

    let done = stream("PIPELINE_DONE");
    assert_eq!(done.map(|done| done.discard), Some(DiscardPolicy::Old));
    assert_eq!(
        done.map(|done| done.max_bytes),
        Some(24 * 1024 * 1024 * 1024)
    );
    assert_eq!(
        done.map(|done| done.max_age),
        Some(Duration::from_secs(72 * 60 * 60))
    );
    assert_eq!(
        done.map(|done| done.duplicate_window),
        Some(Duration::from_secs(12 * 60 * 60))
    );
    for name in [
        "INDEX_AUDIO",
        "EMBED_LYRICS",
        "TRANSCRIBE",
        "ENCODE",
        "TRAIN_COLLAB",
        "TRAIN_TASTE",
        "AI_RPC",
        "WORKER_INVALID",
    ] {
        let spec = stream(name);
        assert_eq!(
            spec.map(|spec| spec.discard),
            Some(DiscardPolicy::New),
            "{name}"
        );
        assert_eq!(
            spec.map(|spec| spec.max_bytes),
            Some(1024 * 1024 * 1024),
            "{name}"
        );
    }
    assert_eq!(
        stream("WORKER_INVALID").map(|invalid| invalid.retention),
        Some(RetentionPolicy::Limits)
    );
    assert!(stream("TRAIN_QUALITY").is_none());
    assert!(RETIRED_STREAMS.contains(&"TRAIN_QUALITY"));
    assert!(
        streams
            .iter()
            .all(|stream| stream.max_message_size == 900_000)
    );
}

#[test]
fn a_request_deadline_is_the_epoch_millisecond_the_caller_stops_waiting() {
    let now = UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);

    assert_eq!(
        deadline_header(now, Duration::from_secs(20)),
        "1700000020123"
    );
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn a_bucket_lists_its_live_objects_with_their_age() -> anyhow::Result<()> {
    let config = NatsConfig {
        url: std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned()),
        job_ingress_concurrency: 1,
        impression_concurrency: 1,
        retry_delay: Duration::from_millis(100),
        max_age: Duration::from_secs(60),
        job_stream_max_bytes: 16 * 1024 * 1024,
        impression_stream_max_bytes: 16 * 1024 * 1024,
    };
    let bus = Bus::connect(&config, "object-listing-test").await?;
    const BUCKET: &str = "OBJECT_LISTING_TEST";
    let _ = bus.jetstream.delete_object_store(BUCKET).await;
    let store = bus
        .jetstream
        .create_object_store(async_nats::jetstream::object_store::Config {
            bucket: BUCKET.to_owned(),
            ..Default::default()
        })
        .await?;
    let empty = bus.list_objects(BUCKET).await?;
    store.put("kept", &mut b"one".as_slice()).await?;
    store.put("gone", &mut b"two".as_slice()).await?;
    bus.delete_object(BUCKET, "gone").await?;
    let started = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    let listed = bus.list_objects(BUCKET).await?;
    bus.jetstream.delete_object_store(BUCKET).await?;

    assert!(empty.is_empty());
    assert_eq!(
        listed
            .iter()
            .map(|object| object.name.as_str())
            .collect::<Vec<_>>(),
        vec!["kept"]
    );
    assert!(listed.iter().all(|object| {
        object
            .modified_unix
            .is_some_and(|at| at.abs_diff(i64::try_from(started).unwrap_or(i64::MAX)) < 60)
    }));
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn explicit_pipeline_duplicate_window_round_trips() -> anyhow::Result<()> {
    let config = NatsConfig {
        url: std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned()),
        job_ingress_concurrency: 1,
        impression_concurrency: 1,
        retry_delay: Duration::from_millis(100),
        max_age: Duration::from_secs(60),
        job_stream_max_bytes: 16 * 1024 * 1024,
        impression_stream_max_bytes: 16 * 1024 * 1024,
    };
    let bus = Bus::connect(&config, "pipeline-contract-test").await?;
    const NAME: &str = "PIPELINE_CONTRACT_TEST";
    const SUBJECT: &str = "pipeline.contract.test";
    let spec = PipelineStreamSpec {
        name: NAME,
        subjects: &[SUBJECT],
        work_queue: false,
        max_age_seconds: 24 * 60 * 60,
        duplicate_window_seconds: 24 * 60 * 60,
        max_bytes: 16 * 1024 * 1024,
        discard: StreamDiscard::Old,
    };
    let desired = pipeline_stream_config(&spec);

    let _ = bus.jetstream.delete_stream(NAME).await;
    let stream = bus.ensure_stream(desired.clone()).await?;
    validate_stream(stream.cached_info().config.clone(), &desired)?;
    bus.jetstream.delete_stream(NAME).await?;
    Ok(())
}

#[test]
fn managed_stream_update_preserves_unowned_settings() {
    let current = StreamConfig {
        name: "stream".to_owned(),
        description: Some("owned by operations".to_owned()),
        max_bytes: 1,
        ..Default::default()
    };
    let desired = StreamConfig {
        name: "stream".to_owned(),
        subjects: vec!["subject".to_owned()],
        retention: RetentionPolicy::WorkQueue,
        max_bytes: 2,
        ..Default::default()
    };

    let updated = managed_stream_config(current, &desired);

    assert_eq!(updated.description.as_deref(), Some("owned by operations"));
    assert_eq!(updated.subjects, desired.subjects);
    assert_eq!(updated.max_bytes, 2);
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn impression_delivery_is_durable_and_acknowledged() -> anyhow::Result<()> {
    let config = NatsConfig {
        url: std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned()),
        job_ingress_concurrency: 1,
        impression_concurrency: 1,
        retry_delay: Duration::from_millis(100),
        max_age: Duration::from_secs(60 * 60),
        job_stream_max_bytes: 16 * 1024 * 1024,
        impression_stream_max_bytes: 16 * 1024 * 1024,
    };
    let bus = Bus::connect(&config, "integration-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let stream_name = format!("JOBS_TEST_{suffix}");
    let dead_letter_stream = format!("JOBS_TEST_DLQ_{suffix}");
    let subject = format!("test.jobs.impressions.{suffix}");
    let dead_letter_subject = format!("test.jobs.dead.{suffix}");
    let durable = format!("jobs-test-{suffix}");
    let stream = bus
        .ensure_work_stream(
            &stream_name,
            &subject,
            config.impression_stream_max_bytes,
            config.max_age,
        )
        .await?;
    bus.ensure_stream(StreamConfig {
        name: dead_letter_stream.clone(),
        subjects: vec![dead_letter_subject.clone()],
        retention: RetentionPolicy::Limits,
        discard: DiscardPolicy::New,
        storage: StorageType::File,
        max_bytes: config.impression_stream_max_bytes,
        max_age: config.max_age,
        max_message_size: MAX_MESSAGE_BYTES,
        duplicate_window: DUPLICATE_WINDOW.min(config.max_age),
        ..Default::default()
    })
    .await?;
    let impressions = bus
        .consumer(
            stream,
            &durable,
            &subject,
            &dead_letter_subject,
            1,
            config.retry_delay,
        )
        .await?;
    let request_id = Uuid::now_v7();
    let batch = ImpressionBatch {
        request_id,
        impressions: vec![Impression {
            impression_id: Uuid::now_v7(),
            user_id: "integration-user".to_owned(),
            track_id: "integration-track".to_owned(),
            cluster_id: "integration-cluster".to_owned(),
            position: 0,
            score: Some(0.5),
            features: None,
            source: "home".to_owned(),
            shown_at_unix_ms: 1,
        }],
    };
    let cancellation = CancellationToken::new();
    let (delivered, received) = oneshot::channel();
    let delivered = Arc::new(Mutex::new(Some(delivered)));
    let task_cancellation = cancellation.clone();
    let task = tokio::spawn(
        impressions.run(task_cancellation, move |actual: ImpressionBatch| {
            let delivered = delivered.clone();
            async move {
                if actual.request_id == request_id
                    && let Some(delivered) = delivered.lock().await.take()
                {
                    let _ = delivered.send(());
                }
                Ok(())
            }
        }),
    );

    let mut headers = HeaderMap::new();
    headers.insert("Nats-Msg-Id", request_id.to_string());
    let payload = serde_json::to_vec(&Versioned::V1(batch))?;
    bus.jetstream
        .publish_with_headers(subject, headers, payload.into())
        .await?
        .await?;
    tokio::time::timeout(Duration::from_secs(5), received).await??;
    tokio::time::sleep(Duration::from_millis(100)).await;
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), task).await???;
    bus.jetstream.delete_stream(stream_name).await?;
    bus.jetstream.delete_stream(dead_letter_stream).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn a_message_whose_handler_failed_comes_back_and_stops_once_it_succeeds() -> anyhow::Result<()>
{
    let config = NatsConfig {
        url: std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned()),
        job_ingress_concurrency: 1,
        impression_concurrency: 1,
        retry_delay: Duration::from_millis(200),
        max_age: Duration::from_secs(60),
        job_stream_max_bytes: 16 * 1024 * 1024,
        impression_stream_max_bytes: 16 * 1024 * 1024,
    };
    let bus = Bus::connect(&config, "redelivery-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let stream_name = format!("JOBS_REDELIVERY_{suffix}");
    let dead_letter_stream = format!("JOBS_REDELIVERY_DLQ_{suffix}");
    let subject = format!("test.jobs.redelivery.{suffix}");
    let dead_letter_subject = format!("test.jobs.redelivery_dead.{suffix}");
    let durable = format!("jobs-redelivery-{suffix}");
    let stream = bus
        .ensure_work_stream(
            &stream_name,
            &subject,
            config.job_stream_max_bytes,
            config.max_age,
        )
        .await?;
    bus.ensure_stream(StreamConfig {
        name: dead_letter_stream.clone(),
        subjects: vec![dead_letter_subject.clone()],
        retention: RetentionPolicy::Limits,
        discard: DiscardPolicy::New,
        storage: StorageType::File,
        max_bytes: config.job_stream_max_bytes,
        max_age: config.max_age,
        max_message_size: MAX_MESSAGE_BYTES,
        duplicate_window: DUPLICATE_WINDOW.min(config.max_age),
        ..Default::default()
    })
    .await?;
    let consumer = bus
        .consumer(
            stream,
            &durable,
            &subject,
            &dead_letter_subject,
            1,
            config.retry_delay,
        )
        .await?;
    bus.publish(&subject, &"once").await?;

    let deliveries = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = deliveries.clone();
    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let task = tokio::spawn(consumer.run_raw(task_cancellation, move |payload: String| {
        let seen = seen.clone();
        async move {
            assert_eq!(payload, "once");
            let attempt = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            if attempt == 1 {
                return Err(JobError::retryable(anyhow::anyhow!("first delivery fails")));
            }
            Ok(())
        }
    }));

    let redelivered = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if deliveries.load(std::sync::atomic::Ordering::SeqCst) >= 2 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        redelivered.is_ok(),
        "a message whose handler failed must be delivered again, saw {} delivery",
        deliveries.load(std::sync::atomic::Ordering::SeqCst)
    );

    tokio::time::sleep(config.retry_delay * 5).await;
    assert_eq!(
        deliveries.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "once the handler succeeded the message must be acknowledged and never come back"
    );

    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), task).await???;
    bus.jetstream.delete_stream(stream_name).await?;
    bus.jetstream.delete_stream(dead_letter_stream).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn delayed_retry_releases_consumer_capacity_immediately() -> anyhow::Result<()> {
    let config = NatsConfig {
        url: std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned()),
        job_ingress_concurrency: 1,
        impression_concurrency: 1,
        retry_delay: Duration::from_secs(2),
        max_age: Duration::from_secs(60),
        job_stream_max_bytes: 16 * 1024 * 1024,
        impression_stream_max_bytes: 16 * 1024 * 1024,
    };
    let bus = Bus::connect(&config, "retry-capacity-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let stream_name = format!("JOBS_RETRY_TEST_{suffix}");
    let dead_letter_stream = format!("JOBS_RETRY_TEST_DLQ_{suffix}");
    let subject = format!("test.jobs.retry.{suffix}");
    let dead_letter_subject = format!("test.jobs.retry_dead.{suffix}");
    let durable = format!("jobs-retry-test-{suffix}");
    let stream = bus
        .ensure_work_stream(
            &stream_name,
            &subject,
            config.job_stream_max_bytes,
            config.max_age,
        )
        .await?;
    bus.ensure_stream(StreamConfig {
        name: dead_letter_stream.clone(),
        subjects: vec![dead_letter_subject.clone()],
        retention: RetentionPolicy::Limits,
        discard: DiscardPolicy::New,
        storage: StorageType::File,
        max_bytes: config.job_stream_max_bytes,
        max_age: config.max_age,
        max_message_size: MAX_MESSAGE_BYTES,
        duplicate_window: DUPLICATE_WINDOW.min(config.max_age),
        ..Default::default()
    })
    .await?;
    let consumer = bus
        .consumer(
            stream,
            &durable,
            &subject,
            &dead_letter_subject,
            1,
            config.retry_delay,
        )
        .await?;
    bus.publish(&subject, &"blocked").await?;
    bus.publish(&subject, &"next").await?;

    let cancellation = CancellationToken::new();
    let (delivered, mut received) = mpsc::unbounded_channel();
    let task_cancellation = cancellation.clone();
    let task = tokio::spawn(consumer.run_raw(task_cancellation, move |payload: String| {
        let delivered = delivered.clone();
        async move {
            let _ = delivered.send(payload.clone());
            if payload == "blocked" {
                return Err(JobError::retryable(anyhow::anyhow!("retry probe")));
            }
            Ok(())
        }
    }));

    let first = tokio::time::timeout(Duration::from_secs(5), received.recv())
        .await?
        .ok_or_else(|| anyhow::anyhow!("retry probe consumer stopped"))?;
    assert_eq!(first, "blocked");
    let started = tokio::time::Instant::now();
    let next = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let payload = received
                .recv()
                .await
                .ok_or_else(|| anyhow::anyhow!("retry probe consumer stopped"))?;
            if payload == "next" {
                return Ok::<_, anyhow::Error>(payload);
            }
        }
    })
    .await??;
    assert_eq!(next, "next");
    assert!(started.elapsed() < config.retry_delay);
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), task).await???;
    bus.jetstream.delete_stream(stream_name).await?;
    bus.jetstream.delete_stream(dead_letter_stream).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires a local JetStream server"]
async fn consumer_update_preserves_the_delivery_cursor() -> anyhow::Result<()> {
    let config = NatsConfig {
        url: std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned()),
        job_ingress_concurrency: 1,
        impression_concurrency: 1,
        retry_delay: Duration::from_millis(100),
        max_age: Duration::from_secs(60),
        job_stream_max_bytes: 16 * 1024 * 1024,
        impression_stream_max_bytes: 16 * 1024 * 1024,
    };
    let bus = Bus::connect(&config, "consumer-update-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let stream_name = format!("JOBS_CURSOR_TEST_{suffix}");
    let subject = format!("test.jobs.cursor.{suffix}");
    let durable = format!("jobs-cursor-test-{suffix}");
    let stream = bus
        .ensure_work_stream(
            &stream_name,
            &subject,
            config.job_stream_max_bytes,
            config.max_age,
        )
        .await?;
    let consumer = bus
        .consumer(
            stream,
            &durable,
            &subject,
            "test.jobs.cursor_dead",
            1,
            config.retry_delay,
        )
        .await?;
    bus.publish(&subject, &"first").await?;

    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let (delivered, received) = oneshot::channel();
    let delivered = Arc::new(Mutex::new(Some(delivered)));
    let task = tokio::spawn(consumer.run_raw(task_cancellation, move |payload: String| {
        let delivered = delivered.clone();
        async move {
            if payload == "first"
                && let Some(delivered) = delivered.lock().await.take()
            {
                let _ = delivered.send(());
            }
            Ok(())
        }
    }));
    tokio::time::timeout(Duration::from_secs(5), received).await??;

    let stream = bus.jetstream.get_stream(&stream_name).await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let info = stream
                .consumer_info(&durable)
                .await
                .map_err(|error| anyhow::anyhow!("consumer info failed: {error}"))?;
            if info.ack_floor.stream_sequence == 1 {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await??;
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), task).await???;

    bus.publish(&subject, &"second").await?;
    let stream = bus.jetstream.get_stream(&stream_name).await?;
    let _consumer = bus
        .consumer(
            stream,
            &durable,
            &subject,
            "test.jobs.cursor_dead",
            2,
            config.retry_delay,
        )
        .await?;
    let stream = bus.jetstream.get_stream(&stream_name).await?;
    let info = stream.consumer_info(&durable).await?;

    assert_eq!(info.ack_floor.stream_sequence, 1);
    assert_eq!(info.num_pending, 1);
    bus.jetstream.delete_stream(stream_name).await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
#[ignore = "requires a local JetStream server"]
async fn job_ingress_persists_in_the_canonical_queue(pool: PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!(
        "../../../api/migrations/0057_background_jobs.sql"
    ))
    .execute(&pool)
    .await?;
    let config = NatsConfig {
        url: std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned()),
        job_ingress_concurrency: 1,
        impression_concurrency: 1,
        retry_delay: Duration::from_millis(100),
        max_age: Duration::from_secs(60 * 60),
        job_stream_max_bytes: 16 * 1024 * 1024,
        impression_stream_max_bytes: 16 * 1024 * 1024,
    };
    let bus = Bus::connect(&config, "integration-test").await?;
    let suffix = Uuid::now_v7().simple().to_string();
    let stream_name = format!("JOBS_TEST_{suffix}");
    let dead_letter_stream = format!("JOBS_TEST_DLQ_{suffix}");
    let subject = format!("test.jobs.ingress.{suffix}");
    let dead_letter_subject = format!("test.jobs.dead.{suffix}");
    let durable = format!("jobs-test-{suffix}");
    let stream = bus
        .ensure_work_stream(
            &stream_name,
            &subject,
            config.job_stream_max_bytes,
            config.max_age,
        )
        .await?;
    bus.ensure_stream(StreamConfig {
        name: dead_letter_stream.clone(),
        subjects: vec![dead_letter_subject.clone()],
        retention: RetentionPolicy::Limits,
        discard: DiscardPolicy::New,
        storage: StorageType::File,
        max_bytes: config.job_stream_max_bytes,
        max_age: config.max_age,
        max_message_size: MAX_MESSAGE_BYTES,
        duplicate_window: DUPLICATE_WINDOW.min(config.max_age),
        ..Default::default()
    })
    .await?;
    let consumer = bus
        .consumer(
            stream,
            &durable,
            &subject,
            &dead_letter_subject,
            1,
            config.retry_delay,
        )
        .await?;
    let repository = crate::queue::JobRepository::new(pool.clone(), "integration-test".to_owned());
    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let task = tokio::spawn(consumer.run(task_cancellation, move |command: JobCommand| {
        let repository = repository.clone();
        async move { crate::app::accept_job_command(&repository, command).await }
    }));
    let id = Uuid::now_v7();
    let command = JobCommand {
        id,
        kind: JobKind::DiscoverAggregates,
        dedup_key: None,
        enqueue_if_absent: false,
        payload: serde_json::to_value(Versioned::V1(EmptyPayload {}))?,
        priority: 3,
        max_attempts: 4,
        available_at_unix_ms: Utc::now().timestamp_millis(),
    };
    let mut headers = HeaderMap::new();
    headers.insert("Nats-Msg-Id", id.to_string());
    let payload = serde_json::to_vec(&Versioned::V1(command))?;
    bus.jetstream
        .publish_with_headers(subject, headers, payload.into())
        .await?
        .await?;

    let row = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(row) = sqlx::query_as::<_, (String, String)>(
                "SELECT kind, lane FROM background_jobs WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&pool)
            .await?
            {
                return Ok::<_, sqlx::Error>(row);
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await??;

    assert_eq!(
        row,
        ("discover.aggregates".to_owned(), "core_bulk".to_owned())
    );
    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), task).await???;
    bus.jetstream.delete_stream(stream_name).await?;
    bus.jetstream.delete_stream(dead_letter_stream).await?;
    Ok(())
}
