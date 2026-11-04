use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::any;
use backend_contracts::{JobKind, LyricsLookupPayload, Versioned};
use base64::Engine as _;
use catalog_sources::{ExternalFetcher, GeniusCfg, GeniusService, LyricsSources};
use sqlx::PgPool;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::config::LyricsConfig;
use crate::handlers::star_priority::{Tier, Turn};
use crate::queue::{
    ClaimOrder, JobRepository, JobResult, LeasedJob, LookupLane, NewJob, WriteGovernor,
};

const TRACKS: usize = 24;
const MUSIXMATCH: &str = "https://apic-desktop.musixmatch.com/ws/1.1";
const LEASE: Duration = Duration::from_secs(600);

#[derive(Clone)]
struct Gate {
    waiting: Arc<AtomicUsize>,
    pass: Arc<Semaphore>,
}

impl Gate {
    fn new() -> Self {
        Self {
            waiting: Arc::new(AtomicUsize::new(0)),
            pass: Arc::new(Semaphore::new(0)),
        }
    }

    fn release(&self, lookups: usize) {
        self.pass.add_permits(lookups);
    }

    fn waiting(&self) -> usize {
        self.waiting.load(Ordering::SeqCst)
    }

    async fn holds(&self, lookups: usize) -> anyhow::Result<()> {
        let steady = async {
            let mut since = None;
            loop {
                if self.waiting() == lookups {
                    let at = *since.get_or_insert_with(tokio::time::Instant::now);
                    if at.elapsed() >= Duration::from_millis(300) {
                        return;
                    }
                } else {
                    since = None;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), steady)
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "the lane should hold {lookups} lookups, it holds {}",
                    self.waiting()
                )
            })
    }
}

async fn gated_source(State(gate): State<Gate>, headers: HeaderMap) -> (StatusCode, String) {
    let target = headers
        .get("x-target")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| base64::engine::general_purpose::STANDARD.decode(value).ok())
        .and_then(|value| String::from_utf8(value).ok())
        .unwrap_or_default();
    let body = if target.contains("macro.subtitles.get") {
        gate.waiting.fetch_add(1, Ordering::SeqCst);
        if let Ok(permit) = gate.pass.acquire().await {
            permit.forget();
        }
        gate.waiting.fetch_sub(1, Ordering::SeqCst);
        r#"{"message":{"header":{"status_code":200},"body":{"macro_calls":{}}}}"#
    } else if target.contains("token.get") {
        r#"{"message":{"body":{"user_token":"test-token"}}}"#
    } else if target.starts_with("https://lrclib.net/api/search") {
        "[]"
    } else if target.starts_with("https://genius.com/api/search/multi") {
        r#"{"response":{"sections":[]}}"#
    } else {
        "{}"
    };
    (StatusCode::OK, body.to_owned())
}

async fn gated_handler(
    pool: &PgPool,
    gate: Gate,
) -> anyhow::Result<(Arc<super::LyricsLookupHandler>, tokio::task::JoinHandle<()>)> {
    let app = Router::new().route("/", any(gated_source)).with_state(gate);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let http = sc_fingerprint::builder(None)
        .timeout(Duration::from_secs(60))
        .build()?;
    let fetcher = ExternalFetcher::new(http, format!("http://{address}/"), None);
    let genius = GeniusService::new(
        fetcher.clone(),
        GeniusCfg {
            access_token: String::new(),
            max_concurrent_scrapes: 8,
        },
    );
    let sources = LyricsSources::new(fetcher, genius, MUSIXMATCH.to_owned());
    let handler = super::LyricsLookupHandler::new(
        pool.clone(),
        sources,
        LyricsConfig {
            batch: 24,
            concurrency: 8,
            claim_seconds: 900,
            backfill_batch: 100,
            musixmatch_base: MUSIXMATCH.to_owned(),
        },
    );
    Ok((Arc::new(handler), server))
}

async fn due_tracks(pool: &PgPool) -> anyhow::Result<()> {
    for id in 1..=TRACKS {
        sqlx::query(
            "INSERT INTO tracks (
                sc_track_id, urn, title, title_normalized, uploader_username, duration_ms, index_priority
             ) VALUES ($1, 'soundcloud:tracks:' || $1, 'Track ' || $1, 'track ' || $1, 'artist', 180000, 5)",
        )
        .bind(id.to_string())
        .execute(pool)
        .await?;
    }
    sqlx::query("UPDATE lyrics_lookup_state SET next_run_at = now() - interval '1 hour'")
        .execute(pool)
        .await?;
    Ok(())
}

async fn sweep_job(pool: &PgPool) -> anyhow::Result<LeasedJob> {
    let queue = JobRepository::new(pool.clone(), "lyrics-sweep-slots".to_owned());
    queue
        .enqueue(&NewJob {
            id: uuid::Uuid::now_v7(),
            kind: JobKind::LyricsLookupSweep,
            dedup_key: None,
            payload: serde_json::json!({}),
            priority: 0,
            max_attempts: 8,
            available_at: chrono::Utc::now() - chrono::Duration::minutes(1),
        })
        .await?;
    queue
        .claim(
            &[JobKind::LyricsLookupSweep],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("sweep job was not claimed"))
}

fn lane_at(limit: usize) -> (Arc<WriteGovernor>, Arc<LookupLane>) {
    let governor = WriteGovernor::new(32);
    governor.hold_at(limit);
    let lane = LookupLane::new(8, governor.clone());
    (governor, lane)
}

async fn wake(pool: &PgPool, sc_track_ids: &[String]) -> anyhow::Result<()> {
    let queue = JobRepository::new(pool.clone(), "lyrics-wakes".to_owned());
    super::super::wake::enqueue_many(pool, &queue, sc_track_ids).await?;
    Ok(())
}

async fn start_targeted(
    lane: &Arc<LookupLane>,
    pool: &PgPool,
    handler: &Arc<super::LyricsLookupHandler>,
    running: &mut JoinSet<JobResult>,
) -> anyhow::Result<usize> {
    let slots = lane.targeted_slots();
    if slots.is_empty() {
        return Ok(0);
    }
    let queue = JobRepository::new(pool.clone(), "lookup-lane".to_owned());
    let claimed = lane.claim_targeted(&queue, slots, LEASE).await?;
    let started = claimed.len();
    for (job, slot) in claimed {
        let payload =
            serde_json::from_value::<Versioned<LyricsLookupPayload>>(job.payload.clone())?
                .into_latest();
        let handler = handler.clone();
        running.spawn(async move {
            let looked_up = handler.run_targeted(&job, payload).await;
            drop(slot);
            looked_up
        });
    }
    Ok(started)
}

async fn not_found(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM lyrics_lookup_state WHERE status = 'not_found'")
            .fetch_one(pool)
            .await?,
    )
}

async fn bulk_jobs(pool: &PgPool, jobs: usize) -> anyhow::Result<()> {
    let queue = JobRepository::new(pool.clone(), "core-bulk".to_owned());
    for job in 0..jobs {
        queue
            .enqueue(&NewJob {
                id: uuid::Uuid::now_v7(),
                kind: JobKind::IndexTrack,
                dedup_key: Some(format!("bulk-{job}")),
                payload: serde_json::json!({}),
                priority: 5,
                max_attempts: 8,
                available_at: chrono::Utc::now() - chrono::Duration::minutes(1),
            })
            .await?;
    }
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn targeted_lookups_in_flight_follow_the_governor_limit(pool: PgPool) -> anyhow::Result<()> {
    due_tracks(&pool).await?;
    let tracks: Vec<String> = (1..=TRACKS).map(|id| id.to_string()).collect();
    wake(&pool, &tracks).await?;
    let gate = Gate::new();
    let (handler, server) = gated_handler(&pool, gate.clone()).await?;
    let (governor, lane) = lane_at(16);
    let mut running = JoinSet::new();

    let phases = async {
        anyhow::ensure!(lane.cap() == 4);
        anyhow::ensure!(start_targeted(&lane, &pool, &handler, &mut running).await? == 4);
        gate.holds(4).await?;
        anyhow::ensure!(start_targeted(&lane, &pool, &handler, &mut running).await? == 0);

        governor.hold_at(32);
        anyhow::ensure!(start_targeted(&lane, &pool, &handler, &mut running).await? == 4);
        gate.holds(8).await?;

        governor.hold_at(8);
        anyhow::ensure!(lane.cap() == 2);
        anyhow::ensure!(start_targeted(&lane, &pool, &handler, &mut running).await? == 0);
        gate.release(7);
        for _ in 0..7 {
            if let Some(done) = running.join_next().await {
                done??;
            }
        }
        anyhow::ensure!(start_targeted(&lane, &pool, &handler, &mut running).await? == 1);
        gate.holds(2).await
    }
    .await;
    gate.release(TRACKS);
    while let Some(done) = running.join_next().await {
        done??;
    }
    server.abort();
    phases?;

    assert_eq!(not_found(&pool).await?, 9);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn waiting_targeted_lookups_cut_the_sweep_to_a_quarter_and_the_bulk_lane_keeps_its_slots(
    pool: PgPool,
) -> anyhow::Result<()> {
    due_tracks(&pool).await?;
    let job = sweep_job(&pool).await?;
    let claims = super::tiered::claim(&pool, &job, 12, 900, Turn::led_by(Tier::StarLiked)).await?;
    assert_eq!(claims.len(), 12);
    let swept: Vec<&str> = claims
        .iter()
        .map(|claim| claim.sc_track_id.as_str())
        .collect();
    let targeted: Vec<String> = (1..=TRACKS)
        .map(|id| id.to_string())
        .filter(|id| !swept.contains(&id.as_str()))
        .collect();
    wake(&pool, &targeted).await?;
    bulk_jobs(&pool, 40).await?;
    let gate = Gate::new();
    let (handler, server) = gated_handler(&pool, gate.clone()).await?;
    let (governor, lane) = lane_at(32);
    let mut running = JoinSet::new();

    let phases = async {
        let checked = async {
            gate.holds(7).await?;
            anyhow::ensure!(start_targeted(&lane, &pool, &handler, &mut running).await? == 1);
            gate.holds(8).await?;

            gate.release(5);
            gate.holds(3).await?;
            anyhow::ensure!(start_targeted(&lane, &pool, &handler, &mut running).await? == 5);
            gate.holds(8).await?;

            let bulk = JobRepository::new(pool.clone(), "core-bulk".to_owned())
                .claim(
                    crate::handlers::CORE_BULK_KINDS,
                    ClaimOrder::Priority,
                    governor.limit(),
                    LEASE,
                )
                .await?;
            anyhow::ensure!(bulk.len() == governor.limit());
            anyhow::ensure!(
                bulk.iter()
                    .all(|job| !crate::handlers::LOOKUP_KINDS.contains(&job.kind))
            );
            gate.holds(8).await
        }
        .await;
        gate.release(TRACKS * 2);
        checked
    };
    let (swept, checked) = tokio::join!(handler.process_batch(&job, claims, &lane), phases);
    while let Some(done) = running.join_next().await {
        done??;
    }
    server.abort();
    checked?;
    swept?;

    assert_eq!(not_found(&pool).await?, 18);
    let left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM lyrics_lookup_queue WHERE claim_job_id = $1")
            .bind(job.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(left, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_sweep_dropped_mid_batch_hands_its_slots_and_its_reserve_back(
    pool: PgPool,
) -> anyhow::Result<()> {
    due_tracks(&pool).await?;
    let job = sweep_job(&pool).await?;
    let claims = super::tiered::claim(&pool, &job, 12, 900, Turn::led_by(Tier::StarLiked)).await?;
    let gate = Gate::new();
    let (handler, server) = gated_handler(&pool, gate.clone()).await?;
    let (_governor, lane) = lane_at(32);

    let while_held = tokio::select! {
        _ = handler.process_batch(&job, claims, &lane) => {
            anyhow::bail!("the sweep finished while its lookups were held")
        }
        held = async {
            gate.holds(7).await?;
            anyhow::Ok(lane.targeted_slots().len())
        } => held?,
    };
    let after_drop = lane.targeted_slots().len();
    gate.release(TRACKS);
    server.abort();

    assert_eq!(while_held, 1);
    assert_eq!(after_drop, 8);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_failed_lookup_lets_the_rest_of_the_sweep_batch_finish_and_returns_its_error(
    pool: PgPool,
) -> anyhow::Result<()> {
    due_tracks(&pool).await?;
    let job = sweep_job(&pool).await?;
    let mut claims =
        super::tiered::claim(&pool, &job, 12, 900, Turn::led_by(Tier::StarLiked)).await?;
    claims.insert(
        0,
        super::LookupClaim {
            track_id: uuid::Uuid::now_v7(),
            sc_track_id: "not a track".to_owned(),
            generation: 1,
            input_title: "Track".to_owned(),
            input_artist: "Artist".to_owned(),
            input_duration_ms: 180_000,
            input_genius_song_id: None,
            input_genius_url: None,
            input_release_date: None,
            input_sc_created_at: None,
            miss_streak: 0,
            failure_streak: 0,
        },
    );
    let gate = Gate::new();
    let (handler, server) = gated_handler(&pool, gate.clone()).await?;
    let (_governor, lane) = lane_at(32);

    let phases = async {
        let held = gate.holds(7).await;
        gate.release(TRACKS);
        held
    };
    let (swept, held) = tokio::join!(handler.process_batch(&job, claims, &lane), phases);
    server.abort();
    held?;

    assert!(swept.is_err());
    assert_eq!(not_found(&pool).await?, 12);
    Ok(())
}
