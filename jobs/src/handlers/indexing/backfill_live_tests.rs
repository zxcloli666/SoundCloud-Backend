use std::sync::Arc;
use std::time::Duration;

use backend_contracts::pipeline::{
    INDEX_AUDIO_STREAM, STORAGE_EVENTS_STREAM, StorageTrackUploaded,
};
use backend_contracts::{StoredAudioDispatchPayload, Versioned};
use tokio_util::sync::CancellationToken;

use crate::config::{DurationConfig, IndexingConfig, NatsConfig, QdrantConfig};

use super::*;

const AUDIO_BACKLOG: i64 = 4;
const STORED_TRACKS: i64 = 10;
const URGENT_PRIORITY: i16 = 0;
const BACKLOG_PRIORITY: i16 = 5;

fn nats_config() -> NatsConfig {
    NatsConfig {
        url: std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned()),
        job_ingress_concurrency: 1,
        impression_concurrency: 1,
        retry_delay: Duration::from_millis(100),
        max_age: Duration::from_secs(60 * 60),
        job_stream_max_bytes: 16 * 1024 * 1024,
        impression_stream_max_bytes: 16 * 1024 * 1024,
    }
}

fn handler(pool: PgPool, bus: Bus) -> anyhow::Result<IndexingHandler> {
    let local = Url::parse("http://127.0.0.1:9")?;
    let indexing = IndexingConfig {
        streaming_url: local.clone(),
        internal_token: String::new().into(),
    };
    let durations = DurationConfig {
        api_v2_url: local.clone(),
        web_url: local.clone(),
        proxy_url: None,
        proxy_fallback: false,
        batch_size: 1,
        concurrency: 1,
        request_gap: Duration::from_millis(1),
        max_track_duration_ms: 7 * 60 * 1000,
    };
    let qdrant = QdrantProvisioner::connect(&QdrantConfig {
        grpc_url: "http://127.0.0.1:6334".to_owned(),
        api_key: String::new().into(),
    })?;
    IndexingHandler::new(
        pool,
        &indexing,
        &durations,
        &Url::parse("http://storage.test/api")?,
        AUDIO_BACKLOG,
        bus,
        qdrant,
    )
}

fn unique_track_id() -> i64 {
    let spread = Uuid::now_v7().as_u128() % 1_000_000_000_000;
    1_000_000_000_000 + i64::try_from(spread).unwrap_or_default()
}

async fn seed_unannounced_tracks(
    pool: &PgPool,
    first_id: i64,
    index_priority: i16,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms,
             storage_state, s3_verified_at, index_state, index_priority, created_at
         )
         SELECT id::text, 'soundcloud:tracks:' || id, 'Song', 'song', 180000,
                'ok', now(), 'pending', $3, now() - interval '1 hour'
         FROM generate_series($1::bigint, $1::bigint + $2::bigint - 1) AS id",
    )
    .bind(first_id)
    .bind(STORED_TRACKS)
    .bind(index_priority)
    .execute(pool)
    .await?;
    Ok(())
}

async fn count(pool: &PgPool, sql: &str) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>(sql).fetch_one(pool).await?)
}

async fn accepted_uploads(pool: &PgPool) -> anyhow::Result<i64> {
    count(pool, "SELECT count(*) FROM storage_event_state").await
}

async fn queued_dispatches(pool: &PgPool) -> anyhow::Result<i64> {
    count(
        pool,
        "SELECT count(*) FROM background_jobs WHERE kind = 'indexing.dispatch_audio'",
    )
    .await
}

struct Outstanding {
    unaccepted_uploads: u64,
    queued_dispatches: i64,
    worker_pending: u64,
}

impl Outstanding {
    async fn read(bus: &Bus, pool: &PgPool) -> anyhow::Result<Self> {
        Ok(Self {
            unaccepted_uploads: bus.unaccepted_storage_uploads().await?,
            queued_dispatches: queued_dispatches(pool).await?,
            worker_pending: bus.worker_pending(&AUDIO_LANE).await?,
        })
    }

    fn total(&self) -> i64 {
        i64::try_from(self.unaccepted_uploads + self.worker_pending).unwrap_or(i64::MAX)
            + self.queued_dispatches
    }
}

async fn reap(indexing: &IndexingHandler) -> anyhow::Result<()> {
    indexing
        .reap()
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))
}

async fn wait_until_uploads_are_accepted(bus: &Bus) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(10), async {
        while bus.unaccepted_storage_uploads().await? > 0 {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await?
}

async fn run_queued_dispatches(pool: &PgPool, indexing: &IndexingHandler) -> anyhow::Result<()> {
    let jobs = sqlx::query_as::<_, (Uuid, serde_json::Value)>(
        "SELECT id, payload FROM background_jobs WHERE kind = 'indexing.dispatch_audio'",
    )
    .fetch_all(pool)
    .await?;
    for (id, payload) in jobs {
        let payload =
            serde_json::from_value::<Versioned<StoredAudioDispatchPayload>>(payload)?.into_latest();
        indexing
            .dispatch_audio(payload)
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        sqlx::query("DELETE FROM background_jobs WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
#[ignore = "requires a disposable JetStream server: it provisions the real stream names"]
async fn stored_tracks_without_a_storage_event_stay_within_the_audio_backlog_across_reaps(
    pool: PgPool,
) -> anyhow::Result<()> {
    let config = nats_config();
    let bus = Bus::connect(&config, "audio-backfill-test").await?;
    let consumers = Box::pin(bus.provision(&config)).await?;
    let jetstream = async_nats::jetstream::new(async_nats::connect(&config.url).await?);
    for stream in [STORAGE_EVENTS_STREAM.name, INDEX_AUDIO_STREAM.name] {
        jetstream.get_stream(stream).await?.purge().await?;
    }
    let first_id = unique_track_id();
    seed_unannounced_tracks(&pool, first_id, BACKLOG_PRIORITY).await?;
    let indexing = Arc::new(handler(pool.clone(), bus.clone())?);

    reap(&indexing).await?;
    let first = Outstanding::read(&bus, &pool).await?;
    indexing
        .announce_stored(&first_id.to_string())
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let deduplicated = bus.unaccepted_storage_uploads().await?;
    seed_unannounced_tracks(&pool, first_id + STORED_TRACKS, URGENT_PRIORITY).await?;
    reap(&indexing).await?;
    let repeated = Outstanding::read(&bus, &pool).await?;

    let cancellation = CancellationToken::new();
    let uploads = tokio::spawn({
        let indexing = indexing.clone();
        consumers.storage_uploads.run_raw_with_context(
            cancellation.clone(),
            move |upload: StorageTrackUploaded, delivery| {
                let indexing = indexing.clone();
                async move { indexing.accept_storage_upload(upload, delivery).await }
            },
        )
    });
    wait_until_uploads_are_accepted(&bus).await?;
    let accepted = (
        accepted_uploads(&pool).await?,
        Outstanding::read(&bus, &pool).await?,
    );

    reap(&indexing).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let after_acceptance = (
        accepted_uploads(&pool).await?,
        Outstanding::read(&bus, &pool).await?,
    );

    run_queued_dispatches(&pool, &indexing).await?;
    reap(&indexing).await?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let dispatched = (
        accepted_uploads(&pool).await?,
        Outstanding::read(&bus, &pool).await?,
    );

    cancellation.cancel();
    uploads.await??;

    assert_eq!(first.unaccepted_uploads, AUDIO_BACKLOG as u64);
    assert_eq!(first.total(), AUDIO_BACKLOG);
    assert_eq!(deduplicated, AUDIO_BACKLOG as u64);
    assert_eq!(repeated.unaccepted_uploads, AUDIO_BACKLOG as u64);
    assert_eq!(repeated.total(), AUDIO_BACKLOG);
    assert_eq!(accepted.0, AUDIO_BACKLOG);
    assert_eq!(accepted.1.queued_dispatches, AUDIO_BACKLOG);
    assert_eq!(accepted.1.total(), AUDIO_BACKLOG);
    assert_eq!(after_acceptance.0, AUDIO_BACKLOG);
    assert_eq!(after_acceptance.1.total(), AUDIO_BACKLOG);
    assert_eq!(dispatched.0, AUDIO_BACKLOG);
    assert_eq!(dispatched.1.worker_pending, AUDIO_BACKLOG as u64);
    assert_eq!(dispatched.1.total(), AUDIO_BACKLOG);
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM background_jobs WHERE kind = 'indexing.track'"
        )
        .await?,
        0
    );
    Ok(())
}
