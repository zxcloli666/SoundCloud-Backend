use std::sync::{Arc, Mutex};
use std::time::Duration;

use backend_contracts::pipeline::StorageTrackUploaded;
use tokio_util::sync::CancellationToken;

use crate::config::{DurationConfig, IndexingConfig, NatsConfig, QdrantConfig};

use super::*;

fn nats_config(url: String, leaves: &[&str]) -> NatsConfig {
    NatsConfig {
        url,
        job_ingress_concurrency: 1,
        impression_concurrency: 1,
        retry_delay: Duration::from_millis(100),
        max_age: Duration::from_secs(60 * 60),
        job_stream_max_bytes: 16 * 1024 * 1024,
        impression_stream_max_bytes: 16 * 1024 * 1024,
        storage_event_sources: leaves.iter().map(|leaf| (*leaf).to_owned()).collect(),
    }
}

fn core(leaves: &[&str]) -> NatsConfig {
    nats_config(
        std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned()),
        leaves,
    )
}

fn leaf() -> NatsConfig {
    nats_config(
        std::env::var("LEAF_NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4223".to_owned()),
        &[],
    )
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
        4,
        bus,
        qdrant,
    )
}

struct Tracks {
    lost: String,
    lost_again: String,
    lost_later: String,
    via_leaf: String,
    too_fresh: String,
    stored: String,
}

impl Tracks {
    fn new() -> Self {
        let base = 1_000_000_000_000 + (Uuid::now_v7().as_u128() % 1_000_000_000_000) as i64;
        let id = |offset: i64| (base + offset).to_string();
        Self {
            lost: id(0),
            lost_again: id(1),
            lost_later: id(2),
            via_leaf: id(3),
            too_fresh: id(4),
            stored: id(5),
        }
    }
}

async fn seed(
    pool: &PgPool,
    sc_track_id: &str,
    storage_state: &str,
    uploaded_minutes_ago: i64,
    event_minutes_ago: Option<i64>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms,
             storage_state, index_state, created_at
         )
         VALUES ($1, 'soundcloud:tracks:' || $1, 'Song', 'song', 180000,
                 $2, 'pending', now() - interval '1 day')",
    )
    .bind(sc_track_id)
    .bind(storage_state)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO cdn_tracks (id, track_urn, quality, cdn_path, status, created_at, updated_at)
         VALUES ($1, 'soundcloud:tracks:' || $2, 'single', $2 || '.m4a', 'ok',
                 LOCALTIMESTAMP - $3 * interval '1 minute',
                 LOCALTIMESTAMP - $3 * interval '1 minute')",
    )
    .bind(Uuid::now_v7())
    .bind(sc_track_id)
    .bind(uploaded_minutes_ago)
    .execute(pool)
    .await?;
    if let Some(minutes) = event_minutes_ago {
        sqlx::query(
            "INSERT INTO storage_event_state (
                 sc_track_id, stream, stream_sequence, event_published_at,
                 uploaded_generation, updated_at
             )
             VALUES ($1, 'STORAGE_EVENTS', 1, now() - $2 * interval '1 minute',
                     1, now() - $2 * interval '1 minute')",
        )
        .bind(sc_track_id)
        .bind(minutes)
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn storage_state(pool: &PgPool, sc_track_id: &str) -> anyhow::Result<String> {
    Ok(
        sqlx::query_scalar::<_, String>("SELECT storage_state FROM tracks WHERE sc_track_id = $1")
            .bind(sc_track_id)
            .fetch_one(pool)
            .await?,
    )
}

async fn repairs(pool: &PgPool) -> anyhow::Result<Vec<(String, chrono::DateTime<Utc>)>> {
    Ok(sqlx::query_as::<_, (String, chrono::DateTime<Utc>)>(
        "SELECT sc_track_id, repaired_at FROM storage_event_repairs ORDER BY sc_track_id",
    )
    .fetch_all(pool)
    .await?)
}

async fn wait_until_stored(pool: &PgPool, tracks: &[&str]) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let mut stored = true;
            for track in tracks {
                stored &= storage_state(pool, track).await? == "ok";
            }
            if stored {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await?
}

fn delivered(deliveries: &Mutex<Vec<String>>) -> Vec<String> {
    let mut seen = deliveries
        .lock()
        .map(|seen| seen.clone())
        .unwrap_or_default();
    seen.sort();
    seen
}

async fn wait_until_settled(bus: &Bus) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !bus.storage_events_settled().await? {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await?
}

async fn sweep(indexing: &IndexingHandler) -> anyhow::Result<()> {
    Box::pin(indexing.repair_lost_uploads())
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))
}

async fn connect(config: &NatsConfig, name: &str) -> anyhow::Result<Bus> {
    Box::pin(Bus::connect(config, name)).await
}

#[sqlx::test(migrations = "../api/migrations")]
#[ignore = "requires the T12 lab (core sourcing STORAGE_EVENTS from leaf domain main); debug builds need RUST_MIN_STACK=8388608"]
async fn an_upload_without_a_storage_event_is_announced_once(pool: PgPool) -> anyhow::Result<()> {
    tls_common::init_crypto();
    let config = core(&["main"]);
    let bus = connect(&config, "upload-repair-test").await?;
    let consumers = Box::pin(bus.provision(&config)).await?;
    bus.restore_topology().await;
    let topology_intact = bus.is_available().await;

    let tracks = Tracks::new();
    seed(&pool, &tracks.lost, "pending", 60, None).await?;
    seed(&pool, &tracks.lost_again, "missing", 60, Some(180)).await?;
    seed(&pool, &tracks.lost_later, "missing", 60, Some(59)).await?;
    seed(&pool, &tracks.via_leaf, "pending", 60, None).await?;
    seed(&pool, &tracks.too_fresh, "pending", 5, None).await?;
    seed(&pool, &tracks.stored, "ok", 60, None).await?;

    let indexing = Arc::new(handler(pool.clone(), bus.clone())?);
    let deliveries = Arc::new(Mutex::new(Vec::<String>::new()));
    let cancellation = CancellationToken::new();
    let uploads = tokio::spawn({
        let indexing = indexing.clone();
        let deliveries = deliveries.clone();
        consumers.storage_uploads.run_raw_with_context(
            cancellation.clone(),
            move |upload: StorageTrackUploaded, delivery| {
                let indexing = indexing.clone();
                if let Ok(mut seen) = deliveries.lock() {
                    seen.push(upload.sc_track_id.clone());
                }
                async move { indexing.accept_storage_upload(upload, delivery).await }
            },
        )
    });

    let leaf = connect(&leaf(), "upload-repair-leaf").await?;
    leaf.publish_dedup(
        STORAGE_TRACK_UPLOADED,
        &json!({
            "sc_track_id": tracks.via_leaf,
            "storage_url": indexing.storage_audio_url(&tracks.via_leaf)?,
            "quality": "hq",
        }),
        &format!("leaf-upload:{}", tracks.via_leaf),
    )
    .await?;
    wait_until_stored(&pool, &[&tracks.via_leaf]).await?;

    let unheard = core(&["main", "unheard"]);
    let unheard_bus = connect(&unheard, "upload-repair-unheard").await?;
    Box::pin(unheard_bus.provision(&unheard)).await?;
    let unheard_settled = unheard_bus.storage_events_settled().await?;
    sweep(&handler(pool.clone(), unheard_bus)?).await?;
    let repairs_while_a_leaf_is_unheard = repairs(&pool).await?.len();
    Box::pin(bus.provision(&config)).await?;

    wait_until_settled(&bus).await?;
    sweep(&indexing).await?;
    wait_until_stored(&pool, &[&tracks.lost, &tracks.lost_again]).await?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let first = repairs(&pool).await?;
    let delivered_after_first = delivered(&deliveries);

    wait_until_settled(&bus).await?;
    sweep(&indexing).await?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let second = repairs(&pool).await?;
    let delivered_after_second = delivered(&deliveries);

    cancellation.cancel();
    uploads.await??;

    let mut expected = vec![tracks.lost.clone(), tracks.lost_again.clone()];
    expected.sort();
    let repaired: Vec<String> = first.iter().map(|(track, _)| track.clone()).collect();
    let mut expected_deliveries = expected.clone();
    expected_deliveries.push(tracks.via_leaf.clone());
    expected_deliveries.sort();
    assert!(
        topology_intact,
        "the sourced stream must match the jobs contract"
    );
    assert!(
        !unheard_settled,
        "a listed leaf that never answered must hold the sweep"
    );
    assert_eq!(repairs_while_a_leaf_is_unheard, 0);
    assert_eq!(repaired, expected);
    assert_eq!(second, first, "a second sweep must not repair again");
    assert_eq!(delivered_after_second, delivered_after_first);
    assert_eq!(delivered_after_first, expected_deliveries);
    assert_eq!(storage_state(&pool, &tracks.lost_later).await?, "missing");
    assert_eq!(storage_state(&pool, &tracks.too_fresh).await?, "pending");
    assert_eq!(storage_state(&pool, &tracks.stored).await?, "ok");
    Ok(())
}
