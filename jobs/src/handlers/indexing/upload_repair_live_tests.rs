use std::sync::{Arc, Mutex};
use std::time::Duration;

use backend_contracts::pipeline::StorageTrackUploaded;
use sqlx::postgres::PgPoolOptions;
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
    lost_upgrade: String,
    lost_later: String,
    via_leaf: String,
    too_fresh: String,
    too_old: String,
    stored: String,
}

impl Tracks {
    fn new() -> Self {
        let base = 1_000_000_000_000 + (Uuid::now_v7().as_u128() % 1_000_000_000_000) as i64;
        let id = |offset: i64| (base + offset).to_string();
        Self {
            lost: id(0),
            lost_again: id(1),
            lost_upgrade: id(2),
            lost_later: id(3),
            via_leaf: id(4),
            too_fresh: id(5),
            too_old: id(6),
            stored: id(7),
        }
    }
}

struct Upload<'a> {
    sc_track_id: &'a str,
    storage_state: &'a str,
    storage_quality: Option<&'a str>,
    stored_quality: Option<&'a str>,
    uploaded_minutes_ago: i64,
    event_minutes_ago: Option<i64>,
}

async fn pool_in_zone(pool: &PgPool, zone: &'static str) -> anyhow::Result<PgPool> {
    let options = (*pool.connect_options()).clone();
    Ok(PgPoolOptions::new()
        .max_connections(4)
        .after_connect(move |connection, _| {
            Box::pin(async move {
                sqlx::query("SELECT set_config('TimeZone', $1, false)")
                    .bind(zone)
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await?)
}

async fn zone_of(pool: &PgPool) -> anyhow::Result<String> {
    Ok(
        sqlx::query_scalar::<_, String>("SELECT current_setting('TimeZone')")
            .fetch_one(pool)
            .await?,
    )
}

async fn seed(writer: &PgPool, upload: Upload<'_>) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms,
             storage_state, storage_quality, index_state, created_at
         )
         VALUES ($1, 'soundcloud:tracks:' || $1, 'Song', 'song', 180000,
                 $2, $3, 'pending', now() - interval '5 days')",
    )
    .bind(upload.sc_track_id)
    .bind(upload.storage_state)
    .bind(upload.storage_quality)
    .execute(writer)
    .await?;
    sqlx::query(
        "INSERT INTO cdn_tracks (
             id, track_urn, quality, cdn_path, status, stored_quality, created_at, updated_at
         )
         VALUES ($1, 'soundcloud:tracks:' || $2, 'single', $2 || '.m4a', 'ok', $3,
                 (NOW() AT TIME ZONE 'UTC') - $4 * interval '1 minute',
                 (NOW() AT TIME ZONE 'UTC') - $4 * interval '1 minute')",
    )
    .bind(Uuid::now_v7())
    .bind(upload.sc_track_id)
    .bind(upload.stored_quality)
    .bind(upload.uploaded_minutes_ago)
    .execute(writer)
    .await?;
    if let Some(minutes) = upload.event_minutes_ago {
        sqlx::query(
            "INSERT INTO storage_event_state (
                 sc_track_id, stream, stream_sequence, event_published_at,
                 uploaded_generation, updated_at
             )
             VALUES ($1, 'STORAGE_EVENTS', 1, now() - $2 * interval '1 minute',
                     1, now() - $2 * interval '1 minute')",
        )
        .bind(upload.sc_track_id)
        .bind(minutes)
        .execute(writer)
        .await?;
    }
    Ok(())
}

async fn stored_as(pool: &PgPool, sc_track_id: &str) -> anyhow::Result<(String, Option<String>)> {
    Ok(sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT storage_state, storage_quality FROM tracks WHERE sc_track_id = $1",
    )
    .bind(sc_track_id)
    .fetch_one(pool)
    .await?)
}

async fn repairs(pool: &PgPool) -> anyhow::Result<Vec<(String, chrono::DateTime<Utc>)>> {
    Ok(sqlx::query_as::<_, (String, chrono::DateTime<Utc>)>(
        "SELECT sc_track_id, repaired_at FROM storage_event_repairs ORDER BY sc_track_id",
    )
    .fetch_all(pool)
    .await?)
}

async fn wait_until_stored(pool: &PgPool, expected: &[(&str, Option<&str>)]) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let mut stored = true;
            for (track, quality) in expected {
                let (state, stored_quality) = stored_as(pool, track).await?;
                stored &= state == "ok" && stored_quality.as_deref() == *quality;
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

async fn settled(bus: &Bus) -> anyhow::Result<bool> {
    let progress = bus.storage_events_progress().await?;
    Ok(progress.sources_behind == 0 && progress.undelivered == 0)
}

async fn wait_until_settled(bus: &Bus) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !settled(bus).await? {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await?
}

async fn paused_seconds(pool: &PgPool, reason: &str) -> anyhow::Result<Option<f64>> {
    let series = format!("jobs_storage_repair_paused_seconds{{reason=\"{reason}\"}} ");
    Ok(crate::metrics::render(pool, &[]).await.and_then(|body| {
        body.lines()
            .find_map(|line| line.strip_prefix(series.as_str()))
            .and_then(|value| value.trim().parse().ok())
    }))
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
    crate::metrics::init();
    let config = core(&["main"]);
    let bus = connect(&config, "upload-repair-test").await?;
    let consumers = Box::pin(bus.provision(&config)).await?;
    bus.restore_topology().await;
    let topology_intact = bus.is_available().await;

    let writer = pool_in_zone(&pool, "America/New_York").await?;
    let sweeper = pool_in_zone(&pool, "Europe/Moscow").await?;
    let zones = (zone_of(&writer).await?, zone_of(&sweeper).await?);
    let tracks = Tracks::new();
    let upload = |sc_track_id, storage_state, uploaded_minutes_ago, event_minutes_ago| Upload {
        sc_track_id,
        storage_state,
        storage_quality: None,
        stored_quality: None,
        uploaded_minutes_ago,
        event_minutes_ago,
    };
    seed(&writer, upload(&tracks.lost, "pending", 60, None)).await?;
    seed(
        &writer,
        Upload {
            stored_quality: Some("sq"),
            ..upload(&tracks.lost_again, "missing", 60, Some(180))
        },
    )
    .await?;
    seed(
        &writer,
        Upload {
            storage_quality: Some("sq"),
            stored_quality: Some("hq"),
            ..upload(&tracks.lost_upgrade, "ok", 60, Some(180))
        },
    )
    .await?;
    seed(&writer, upload(&tracks.lost_later, "missing", 60, Some(59))).await?;
    seed(&writer, upload(&tracks.via_leaf, "pending", 60, None)).await?;
    seed(&writer, upload(&tracks.too_fresh, "pending", 5, None)).await?;
    seed(
        &writer,
        upload(&tracks.too_old, "pending", 3 * 24 * 60, None),
    )
    .await?;
    seed(
        &writer,
        Upload {
            storage_quality: Some("sq"),
            stored_quality: Some("sq"),
            ..upload(&tracks.stored, "ok", 60, None)
        },
    )
    .await?;

    let indexing = Arc::new(handler(sweeper.clone(), bus.clone())?);
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
    wait_until_stored(&pool, &[(&tracks.via_leaf, Some("hq"))]).await?;

    let unheard = core(&["main", "unheard"]);
    let unheard_bus = connect(&unheard, "upload-repair-unheard").await?;
    Box::pin(unheard_bus.provision(&unheard)).await?;
    let unheard_settled = settled(&unheard_bus).await?;
    sweep(&handler(sweeper.clone(), unheard_bus)?).await?;
    let repairs_while_a_leaf_is_unheard = repairs(&pool).await?.len();
    let paused_while_unheard = paused_seconds(&pool, "leaf_source_behind").await?;
    Box::pin(bus.provision(&config)).await?;

    wait_until_settled(&bus).await?;
    sweep(&indexing).await?;
    let paused_after_a_sweep = paused_seconds(&pool, "leaf_source_behind").await?;
    wait_until_stored(
        &pool,
        &[
            (&tracks.lost, None),
            (&tracks.lost_again, Some("sq")),
            (&tracks.lost_upgrade, Some("hq")),
        ],
    )
    .await?;
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

    let mut expected = vec![
        tracks.lost.clone(),
        tracks.lost_again.clone(),
        tracks.lost_upgrade.clone(),
    ];
    expected.sort();
    let repaired: Vec<String> = first.iter().map(|(track, _)| track.clone()).collect();
    let mut expected_deliveries = expected.clone();
    expected_deliveries.push(tracks.via_leaf.clone());
    expected_deliveries.sort();
    assert_eq!(
        zones,
        ("America/New_York".to_owned(), "Europe/Moscow".to_owned())
    );
    assert!(
        topology_intact,
        "the sourced stream must match the jobs contract"
    );
    assert!(
        !unheard_settled,
        "a listed leaf that never answered must hold the sweep"
    );
    assert_eq!(repairs_while_a_leaf_is_unheard, 0);
    assert!(paused_while_unheard.is_some_and(|seconds| seconds > 0.0));
    assert_eq!(paused_after_a_sweep, Some(0.0));
    assert_eq!(repaired, expected);
    assert_eq!(second, first, "a second sweep must not repair again");
    assert_eq!(delivered_after_second, delivered_after_first);
    assert_eq!(delivered_after_first, expected_deliveries);
    assert_eq!(
        stored_as(&pool, &tracks.lost_later).await?,
        ("missing".to_owned(), None)
    );
    assert_eq!(
        stored_as(&pool, &tracks.too_fresh).await?,
        ("pending".to_owned(), None)
    );
    assert_eq!(
        stored_as(&pool, &tracks.too_old).await?,
        ("pending".to_owned(), None)
    );
    assert_eq!(
        stored_as(&pool, &tracks.stored).await?,
        ("ok".to_owned(), Some("sq".to_owned()))
    );
    Ok(())
}
