use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::any;
use backend_contracts::JobKind;
use base64::Engine as _;
use catalog_sources::{ExternalFetcher, GeniusCfg, GeniusService, LyricsSources};
use sqlx::PgPool;
use tokio::sync::Semaphore;

use crate::config::LyricsConfig;
use crate::handlers::star_priority::{Tier, Turn};
use crate::queue::{ClaimOrder, JobRepository, LeasedJob, NewJob, WriteGovernor};

const TRACKS: usize = 24;
const MUSIXMATCH: &str = "https://apic-desktop.musixmatch.com/ws/1.1";
const DUE_KINDS: &[JobKind] = &[
    JobKind::CatalogCollection,
    JobKind::IndexTrack,
    JobKind::LyricsLookup,
    JobKind::LyricsLookupSweep,
];

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
                    "the sweep should hold {lookups} lookups, it holds {}",
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
) -> anyhow::Result<(super::LyricsLookupHandler, tokio::task::JoinHandle<()>)> {
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
            concurrency: 16,
            claim_seconds: 900,
            backfill_batch: 100,
            musixmatch_base: MUSIXMATCH.to_owned(),
        },
    );
    Ok((handler, server))
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

#[sqlx::test(migrations = "../api/migrations")]
async fn sweep_lookups_take_their_share_of_a_busy_lane_and_shrink_with_the_governor_limit(
    pool: PgPool,
) -> anyhow::Result<()> {
    due_tracks(&pool).await?;
    let job = sweep_job(&pool).await?;
    let claims = super::tiered::claim(
        &pool,
        &job,
        i64::try_from(TRACKS)?,
        900,
        Turn::led_by(Tier::StarLiked),
    )
    .await?;
    assert_eq!(claims.len(), TRACKS);
    let gate = Gate::new();
    let (handler, server) = gated_handler(&pool, gate.clone()).await?;
    let governor = WriteGovernor::new(32);
    governor.hold_at(32);
    governor.lane_round(DUE_KINDS);
    assert_eq!(governor.claim_room(32, 32, 64), 0);
    let borrower = governor.borrower(JobKind::LyricsLookupSweep);
    let within = |lane: usize, limit: usize| {
        let total = lane + gate.waiting().saturating_sub(1);
        anyhow::ensure!(
            total <= limit,
            "lane jobs and sweep lookups hold {total} slots over the limit {limit}"
        );
        Ok(())
    };

    let phases = async {
        let checked = async {
            gate.holds(1).await?;

            anyhow::ensure!(governor.claim_room(25, 32, 64) == 0);
            gate.holds(8).await?;
            anyhow::ensure!(governor.claim_room(25, 32, 64) == 0);
            within(25, 32)?;

            governor.hold_at(8);
            anyhow::ensure!(governor.claim_room(6, 32, 64) == 0);
            gate.release(8);
            gate.holds(2).await?;
            anyhow::ensure!(governor.claim_room(6, 32, 64) == 1);
            within(7, 8)?;

            governor.lane_idle(1);
            gate.holds(7).await?;
            within(1, 8)
        }
        .await;
        gate.release(TRACKS);
        checked
    };
    let (swept, checked) = tokio::join!(handler.process_batch(&job, claims, &borrower), phases);
    server.abort();
    checked?;
    swept?;

    let settled: (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE status = 'not_found'),
                (SELECT count(*) FROM lyrics_lookup_queue WHERE claim_job_id = $1)
         FROM lyrics_lookup_state",
    )
    .bind(job.id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(settled, (i64::try_from(TRACKS)?, 0));
    Ok(())
}
