use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::any;
use base64::Engine as _;
use catalog_sources::{ExternalFetcher, GeniusCfg, GeniusService, LyricsCandidate, LyricsSources};
use chrono::Utc;
use sqlx::PgPool;

use crate::config::LyricsConfig;
use crate::queue::{ClaimOrder, JobRepository, LeasedJob};

async fn insert_track(pool: &PgPool, id: &str, title: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (
            sc_track_id, urn, title, title_normalized, uploader_username, duration_ms, index_priority
         ) VALUES ($1, 'soundcloud:tracks:' || $1, $2, lower($2), 'artist', 180000, 5)",
    )
    .bind(id)
    .bind(title)
    .execute(pool)
    .await?;
    Ok(())
}

const LRCLIB_PLAIN: &str = r#"[{"plainLyrics":"A sufficiently long plain lyrics body from proxy E2E","artistName":"artist","trackName":"Lose Yourself","duration":180}]"#;

const LRCLIB_SYNCED: &str = r#"[{"syncedLyrics":"[00:01.00] A sufficiently long plain lyrics body from proxy E2E","plainLyrics":"A sufficiently long plain lyrics body from proxy E2E","artistName":"artist","trackName":"Lose Yourself","duration":180}]"#;

async fn lookup_proxy(
    lrclib: &'static str,
) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .route("/", any(lookup_proxy_response))
        .with_state((calls.clone(), lrclib));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}/"), calls, server)
}

async fn lookup_proxy_response(
    State((calls, lrclib)): State<(Arc<AtomicUsize>, &'static str)>,
    headers: HeaderMap,
) -> (StatusCode, String) {
    calls.fetch_add(1, Ordering::Relaxed);
    let target = headers
        .get("x-target")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| base64::engine::general_purpose::STANDARD.decode(value).ok())
        .and_then(|value| String::from_utf8(value).ok())
        .unwrap_or_default();
    let body = if target.starts_with("https://lrclib.net/api/search") {
        lrclib
    } else if target.contains("musixmatch.com") && target.contains("token.get") {
        r#"{"message":{"body":{"user_token":"UpgradeOnlyUpgradeOnlyUpgradeOnlyUpgradeOnly"}}}"#
    } else if target.starts_with("https://genius.com/api/search/multi") {
        r#"{"response":{"sections":[]}}"#
    } else {
        "{}"
    };
    (StatusCode::OK, body.to_owned())
}

async fn empty_lookup_proxy() -> (String, tokio::task::JoinHandle<()>) {
    let app = Router::new().route("/", any(empty_lookup_proxy_response));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}/"), server)
}

async fn empty_lookup_proxy_response(headers: HeaderMap) -> (StatusCode, String) {
    let target = headers
        .get("x-target")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| base64::engine::general_purpose::STANDARD.decode(value).ok())
        .and_then(|value| String::from_utf8(value).ok())
        .unwrap_or_default();
    let body = if target.starts_with("https://lrclib.net/api/search") {
        "[]"
    } else if target.contains("musixmatch.com") && target.contains("token.get") {
        r#"{"message":{"body":{"user_token":"test-token"}}}"#
    } else if target.contains("musixmatch.com") && target.contains("macro.subtitles.get") {
        r#"{"message":{"header":{"status_code":200},"body":{"macro_calls":{}}}}"#
    } else if target.starts_with("https://genius.com/api/search/multi") {
        r#"{"response":{"sections":[]}}"#
    } else {
        "{}"
    };
    (StatusCode::OK, body.to_owned())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn new_track_creates_due_lookup_state(pool: PgPool) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Track").await?;

    let state: (String, i64, i16, bool) = sqlx::query_as(
        "SELECT status, generation, priority, wake_message_id IS NOT NULL
         FROM lyrics_lookup_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(state, ("pending".to_owned(), 1, 5, true));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn input_change_reopens_once_and_unrelated_change_does_not(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Track").await?;
    sqlx::query(
        "UPDATE lyrics_lookup_state
         SET status = 'not_found', next_run_at = now() + interval '30 days'",
    )
    .execute(&pool)
    .await?;

    sqlx::query("UPDATE tracks SET metadata_artist = 'Canonical Artist' WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE tracks SET play_count_sc = 10 WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;

    let state: (String, i64, String) = sqlx::query_as(
        "SELECT status, generation, input_artist
         FROM lyrics_lookup_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        state,
        ("pending".to_owned(), 2, "Canonical Artist".to_owned())
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn external_cache_prevents_lookup_state(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, plain_text, source)
         VALUES ('42', 'A sufficiently long external lyrics body', 'genius')",
    )
    .execute(&pool)
    .await?;

    insert_track(&pool, "42", "Track").await?;

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_lookup_state")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn keyset_backfill_is_bounded_and_restart_safe(pool: PgPool) -> anyhow::Result<()> {
    insert_track(&pool, "41", "First").await?;
    insert_track(&pool, "42", "Second").await?;
    sqlx::query("DELETE FROM lyrics_lookup_state")
        .execute(&pool)
        .await?;

    let first = sqlx::query_file_scalar!("queries/lyrics/lookup_backfill.sql", 1_i64)
        .fetch_one(&pool)
        .await?;
    let first_count: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_lookup_state")
        .fetch_one(&pool)
        .await?;
    let second = sqlx::query_file_scalar!("queries/lyrics/lookup_backfill.sql", 1_i64)
        .fetch_one(&pool)
        .await?;
    let second_count: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_lookup_state")
        .fetch_one(&pool)
        .await?;

    assert!(!first);
    assert!(!second);
    assert_eq!(first_count, 1);
    assert_eq!(second_count, 2);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_finished_backfill_is_not_read_again(pool: PgPool) -> anyhow::Result<()> {
    insert_track(&pool, "41", "First").await?;
    let handler = offline_handler(&pool)?;

    handler.backfill().await?;
    handler.backfill().await?;
    sqlx::raw_sql(
        "UPDATE lyrics_lookup_backfill_state SET completed = false, cursor_id = NULL;
         DELETE FROM lyrics_lookup_state;",
    )
    .execute(&pool)
    .await?;
    handler.backfill().await?;

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_lookup_state")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn clean_source_empty_persists_negative_without_worker_jobs(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let queue = JobRepository::new(pool.clone(), "lyrics-empty".to_owned());
    super::super::wake::enqueue(&pool, &queue, "42").await?;
    let mut jobs = queue
        .claim(
            &[backend_contracts::JobKind::LyricsLookup],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?;
    let job = jobs
        .pop()
        .ok_or_else(|| anyhow::anyhow!("lookup job was not claimed"))?;
    let (proxy_url, server) = empty_lookup_proxy().await;
    let http = sc_fingerprint::builder(None)
        .timeout(Duration::from_secs(5))
        .build()?;
    let fetcher = ExternalFetcher::new(http, proxy_url, None);
    let genius = GeniusService::new(
        fetcher.clone(),
        GeniusCfg {
            access_token: String::new(),
            max_concurrent_scrapes: 8,
        },
    );
    let sources = LyricsSources::new(
        fetcher,
        genius,
        "https://apic-desktop.musixmatch.com/ws/1.1".to_owned(),
    );
    let handler = super::LyricsLookupHandler::new(
        pool.clone(),
        sources,
        LyricsConfig {
            batch: 8,
            concurrency: 4,
            claim_seconds: 900,
            backfill_batch: 100,
            musixmatch_base: "https://apic-desktop.musixmatch.com/ws/1.1".to_owned(),
        },
    );

    handler
        .run_targeted(
            &job,
            backend_contracts::LyricsLookupPayload {
                sc_track_id: "42".to_owned(),
            },
        )
        .await?;

    server.abort();
    let state: (String, i32, i32, bool) = sqlx::query_as(
        "SELECT status, failure_streak, miss_streak, next_run_at > now()
         FROM lyrics_lookup_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let cache_count: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_cache")
        .fetch_one(&pool)
        .await?;
    let downstream: i64 =
        sqlx::query_scalar("SELECT count(*) FROM background_jobs WHERE kind <> 'lyrics.lookup'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(state, ("not_found".to_owned(), 0, 1, true));
    assert_eq!(cache_count, 0);
    assert_eq!(downstream, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn source_outage_persists_retry_without_worker_jobs(pool: PgPool) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let queue = JobRepository::new(pool.clone(), "lyrics-outage".to_owned());
    super::super::wake::enqueue(&pool, &queue, "42").await?;
    let mut jobs = queue
        .claim(
            &[backend_contracts::JobKind::LyricsLookup],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?;
    let job = jobs
        .pop()
        .ok_or_else(|| anyhow::anyhow!("lookup job was not claimed"))?;
    let http = sc_fingerprint::builder(None)
        .timeout(Duration::from_secs(1))
        .build()?;
    let fetcher = ExternalFetcher::new(http, String::new(), None);
    let genius = GeniusService::new(
        fetcher.clone(),
        GeniusCfg {
            access_token: String::new(),
            max_concurrent_scrapes: 8,
        },
    );
    let sources = LyricsSources::new(
        fetcher,
        genius,
        "https://apic-desktop.musixmatch.com/ws/1.1".to_owned(),
    );
    let handler = super::LyricsLookupHandler::new(
        pool.clone(),
        sources,
        LyricsConfig {
            batch: 8,
            concurrency: 4,
            claim_seconds: 900,
            backfill_batch: 100,
            musixmatch_base: "https://apic-desktop.musixmatch.com/ws/1.1".to_owned(),
        },
    );

    handler
        .run_targeted(
            &job,
            backend_contracts::LyricsLookupPayload {
                sc_track_id: "42".to_owned(),
            },
        )
        .await?;

    let state: (String, i32, i32, bool) = sqlx::query_as(
        "SELECT status, failure_streak, miss_streak, next_run_at > now()
         FROM lyrics_lookup_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let cache_count: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_cache")
        .fetch_one(&pool)
        .await?;
    let downstream: i64 =
        sqlx::query_scalar("SELECT count(*) FROM background_jobs WHERE kind <> 'lyrics.lookup'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(state, ("retry".to_owned(), 1, 0, true));
    assert_eq!(cache_count, 0);
    assert_eq!(downstream, 0);
    Ok(())
}

async fn lookup_through_proxy(pool: &PgPool, lrclib: &'static str) -> anyhow::Result<usize> {
    insert_track(pool, "42", "Lose Yourself").await?;
    sqlx::query("UPDATE tracks SET storage_state = 'ok' WHERE sc_track_id = '42'")
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO storage_event_state (
             sc_track_id, stream, stream_sequence, event_published_at, uploaded_generation
         ) VALUES ('42', 'storage', 1, now(), 1)",
    )
    .execute(pool)
    .await?;
    let queue = JobRepository::new(pool.clone(), "lyrics-e2e".to_owned());
    super::super::wake::enqueue(pool, &queue, "42").await?;
    let mut jobs = queue
        .claim(
            &[backend_contracts::JobKind::LyricsLookup],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?;
    let job = jobs
        .pop()
        .ok_or_else(|| anyhow::anyhow!("lookup job was not claimed"))?;
    let (proxy_url, calls, server) = lookup_proxy(lrclib).await;
    let http = sc_fingerprint::builder(None)
        .timeout(Duration::from_secs(5))
        .build()?;
    let fetcher = ExternalFetcher::new(http, proxy_url, None);
    let genius = GeniusService::new(
        fetcher.clone(),
        GeniusCfg {
            access_token: String::new(),
            max_concurrent_scrapes: 8,
        },
    );
    let sources = LyricsSources::new(
        fetcher,
        genius,
        "https://apic-desktop.musixmatch.com/ws/1.1".to_owned(),
    );
    let handler = super::LyricsLookupHandler::new(
        pool.clone(),
        sources,
        LyricsConfig {
            batch: 8,
            concurrency: 4,
            claim_seconds: 900,
            backfill_batch: 100,
            musixmatch_base: "https://apic-desktop.musixmatch.com/ws/1.1".to_owned(),
        },
    );

    handler
        .run_targeted(
            &job,
            backend_contracts::LyricsLookupPayload {
                sc_track_id: "42".to_owned(),
            },
        )
        .await?;

    server.abort();
    Ok(calls.load(Ordering::Relaxed))
}

async fn sync_provenance(pool: &PgPool) -> anyhow::Result<(Option<String>, Option<String>)> {
    Ok(sqlx::query_as(
        "SELECT synced_source, synced_version FROM lyrics_cache WHERE sc_track_id = '42'",
    )
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn proxy_fallback_persists_lyrics_and_enqueues_only_align(
    pool: PgPool,
) -> anyhow::Result<()> {
    let calls = lookup_through_proxy(&pool, LRCLIB_PLAIN).await?;

    let lyrics: (
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        String,
    ) = sqlx::query_as(
        "SELECT synced_lrc, plain_text, source, plain_source, embedding_state
         FROM lyrics_cache WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let state_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM lyrics_lookup_state WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    let downstream: Vec<String> = sqlx::query_scalar(
        "SELECT kind FROM background_jobs
         WHERE kind <> 'lyrics.lookup'
         ORDER BY kind",
    )
    .fetch_all(&pool)
    .await?;
    assert!(calls >= 3);
    assert_eq!(lyrics.0, None);
    assert!(lyrics.1.is_some());
    assert_eq!(lyrics.2, "lrclib");
    assert_eq!(lyrics.3.as_deref(), Some("lrclib"));
    assert_eq!(lyrics.4, "queued");
    assert_eq!(state_count, 0);
    assert_eq!(
        downstream,
        vec![
            "lyrics.dispatch_transcription".to_owned(),
            "lyrics.embed".to_owned()
        ]
    );
    assert_eq!(sync_provenance(&pool).await?, (None, None));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_external_sync_is_labelled_with_its_source(pool: PgPool) -> anyhow::Result<()> {
    lookup_through_proxy(&pool, LRCLIB_SYNCED).await?;

    let synced: Option<String> =
        sqlx::query_scalar("SELECT synced_lrc FROM lyrics_cache WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert!(synced.is_some_and(|lyrics| lyrics.starts_with("[00:01.00]")));
    assert_eq!(
        sync_provenance(&pool).await?,
        (Some("lrclib".to_owned()), None)
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn immediate_wake_uses_state_message_identity(pool: PgPool) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Track").await?;
    let queue = JobRepository::new(pool.clone(), "lookup-test".to_owned());

    super::super::wake::enqueue(&pool, &queue, "42").await?;
    super::super::wake::enqueue(&pool, &queue, "42").await?;

    let job: (uuid::Uuid, String, String) = sqlx::query_as(
        "SELECT job.id, job.kind, job.dedup_key
         FROM background_jobs AS job WHERE job.kind = 'lyrics.lookup'",
    )
    .fetch_one(&pool)
    .await?;
    let state: (uuid::Uuid, bool) = sqlx::query_as(
        "SELECT wake_message_id, wake_durable_at IS NOT NULL
         FROM lyrics_lookup_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(job.0, state.0);
    assert_eq!(job.1, "lyrics.lookup");
    assert_eq!(job.2, "42");
    assert!(state.1);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn due_wakes_become_one_lookup_job_each_and_a_claimed_track_is_left_alone(
    pool: PgPool,
) -> anyhow::Result<()> {
    for id in ["41", "42", "43"] {
        insert_track(&pool, id, "Track").await?;
    }
    sqlx::query(
        "UPDATE lyrics_lookup_queue AS queue
         SET claim_job_id = gen_random_uuid(), claim_job_generation = 1,
             claim_job_lease_id = gen_random_uuid(), claim_state_generation = state.generation,
             claim_expires_at = now() + interval '1 hour'
         FROM lyrics_lookup_state AS state
         WHERE state.track_id = queue.track_id
           AND state.sc_track_id = '43'",
    )
    .execute(&pool)
    .await?;
    let queue = JobRepository::new(pool.clone(), "lookup-test".to_owned());

    super::super::wake::enqueue_due(&pool, &queue, 10).await?;
    super::super::wake::enqueue_due(&pool, &queue, 10).await?;

    let jobs: Vec<String> = sqlx::query_scalar(
        "SELECT dedup_key FROM background_jobs WHERE kind = 'lyrics.lookup' ORDER BY dedup_key",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(jobs, ["41", "42"]);
    let durable: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM lyrics_lookup_state WHERE wake_durable_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(durable, 2);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_notify_on_the_wake_channel_makes_the_lookup_job_within_a_second(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "51", "Track").await?;
    let queue = JobRepository::new(pool.clone(), "lookup-test".to_owned());
    let cancellation = tokio_util::sync::CancellationToken::new();
    let listener = tokio::spawn(super::super::wake::listen(
        pool.clone(),
        queue,
        cancellation.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(500)).await;

    sqlx::query("SELECT pg_notify('lyrics_wake', '51')")
        .execute(&pool)
        .await?;

    let started = std::time::Instant::now();
    loop {
        let durable: bool = sqlx::query_scalar(
            "SELECT wake_durable_at IS NOT NULL FROM lyrics_lookup_state WHERE sc_track_id = '51'",
        )
        .fetch_one(&pool)
        .await?;
        if durable {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "no job after notify"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let jobs: i64 =
        sqlx::query_scalar("SELECT count(*) FROM background_jobs WHERE kind = 'lyrics.lookup'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(jobs, 1);
    cancellation.cancel();
    listener.await?;
    Ok(())
}

#[tokio::test]
#[ignore = "needs a real api node and core linked by logical replication: SCD_API_URL, SCD_CORE_URL"]
async fn wake_from_an_api_node_reaches_core_through_the_apply_worker() -> anyhow::Result<()> {
    let api = PgPool::connect(&std::env::var("SCD_API_URL")?).await?;
    let core = PgPool::connect(&std::env::var("SCD_CORE_URL")?).await?;
    let queue = JobRepository::new(core.clone(), "lookup-test".to_owned());
    let cancellation = tokio_util::sync::CancellationToken::new();
    let listener = tokio::spawn(super::super::wake::listen(
        core.clone(),
        queue,
        cancellation.clone(),
    ));
    tokio::time::sleep(Duration::from_secs(1)).await;
    let mut millis = Vec::new();
    for number in 0..50 {
        let id = format!("9{number:03}");
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, uploader_username, duration_ms, index_priority)
             VALUES ($1, 'soundcloud:tracks:' || $1, 'Track', 'track', 'artist', 180000, 5)",
        )
        .bind(&id)
        .execute(&core)
        .await?;
        let started = std::time::Instant::now();
        loop {
            let found: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM tracks WHERE sc_track_id = $1)")
                    .bind(&id)
                    .fetch_one(&api)
                    .await?;
            if found {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "track never replicated"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let at = Utc::now();
        let started = std::time::Instant::now();
        sqlx::query("SELECT scd_wt('lyrics.wake', jsonb_build_object('sc_track_id', $1::text, 'at', $2::timestamptz))")
            .bind(&id)
            .bind(at)
            .execute(&api)
            .await?;
        loop {
            let durable: Option<bool> = sqlx::query_scalar(
                "SELECT wake_durable_at IS NOT NULL FROM lyrics_lookup_state WHERE sc_track_id = $1",
            )
            .bind(&id)
            .fetch_optional(&core)
            .await?;
            if durable == Some(true) {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "wake never reached core"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        millis.push(started.elapsed().as_millis());
    }
    cancellation.cancel();
    listener.await?;
    millis.sort_unstable();
    println!(
        "api call to published wake over 50 calls: p50 {} ms, max {} ms",
        millis[25], millis[49]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_sweep_claims_star_likes_likes_star_tracks_the_old_order_and_the_oldest_share(
    pool: PgPool,
) -> anyhow::Result<()> {
    for id in ["45", "44", "43", "42", "41"] {
        insert_track(&pool, id, "Track").await?;
    }
    sqlx::raw_sql(
        "UPDATE tracks SET index_priority = 0 WHERE sc_track_id = '41';
         UPDATE tracks SET index_priority = 1 WHERE sc_track_id = '42';
         INSERT INTO star_track_tiers (sc_track_id, tier) VALUES ('43', 3), ('44', 1);
         UPDATE lyrics_lookup_state SET next_run_at = now() - interval '1 hour';
         UPDATE lyrics_lookup_state SET next_run_at = now() - interval '2 hours'
         WHERE sc_track_id = '45';",
    )
    .execute(&pool)
    .await?;
    let job = sweep_job(&pool).await?;
    let versions = state_versions(&pool).await?;

    let claims = super::tiered::claim(
        &pool,
        &job,
        5,
        900,
        crate::handlers::star_priority::Turn::led_by(
            crate::handlers::star_priority::Tier::StarLiked,
        ),
    )
    .await?;

    let claimed: Vec<String> = claims.into_iter().map(|claim| claim.sc_track_id).collect();
    assert_eq!(claimed, ["44", "42", "43", "41", "45"]);
    assert_eq!(state_versions(&pool).await?, versions);
    let held: i64 =
        sqlx::query_scalar("SELECT count(*) FROM lyrics_lookup_queue WHERE claim_job_id = $1")
            .bind(job.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(held, 5);
    let next = sqlx::query_file_scalar!("queries/lyrics/lookup_next_oldest.sql")
        .fetch_optional(&pool)
        .await?;
    assert_eq!(next, None);
    Ok(())
}

async fn state_versions(pool: &PgPool) -> anyhow::Result<Vec<(String, String)>> {
    Ok(sqlx::query_as(
        "SELECT sc_track_id, ctid::text || ':' || xmin::text
         FROM lyrics_lookup_state ORDER BY sc_track_id",
    )
    .fetch_all(pool)
    .await?)
}

async fn sweep_job(pool: &PgPool) -> anyhow::Result<crate::queue::LeasedJob> {
    let queue = JobRepository::new(pool.clone(), "lyrics-sweep".to_owned());
    queue
        .enqueue(&crate::queue::NewJob {
            id: uuid::Uuid::now_v7(),
            kind: backend_contracts::JobKind::LyricsLookupSweep,
            dedup_key: None,
            payload: serde_json::json!({}),
            priority: 0,
            max_attempts: 8,
            available_at: chrono::Utc::now() - chrono::Duration::minutes(1),
        })
        .await?;
    queue
        .claim(
            &[backend_contracts::JobKind::LyricsLookupSweep],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("sweep job was not claimed"))
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn wake_many_leaves_the_same_jobs_and_marks_as_one_wake_per_track(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE FUNCTION refuse_broken_wake() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN
             IF NEW.dedup_key LIKE '%4' THEN RAISE EXCEPTION 'wake refused'; END IF;
             RETURN NEW;
         END $$;
         CREATE TRIGGER refuse_broken_wake BEFORE INSERT ON background_jobs
         FOR EACH ROW EXECUTE FUNCTION refuse_broken_wake();",
    )
    .execute(&pool)
    .await?;
    for prefix in ["a", "b"] {
        for number in 1..=6 {
            insert_track(&pool, &format!("{prefix}{number}"), "Track").await?;
        }
        sqlx::query(
            "UPDATE lyrics_lookup_state SET wake_durable_at = now() WHERE sc_track_id = $1",
        )
        .bind(format!("{prefix}2"))
        .execute(&pool)
        .await?;
    }
    let queue = JobRepository::new(pool.clone(), "wake-many".to_owned());

    let ids = |prefix: &str| {
        ["1", "2", "3", "4", "5", "6", "missing"]
            .map(|number| format!("{prefix}{number}"))
            .to_vec()
    };
    let many = super::super::wake::enqueue_many(&pool, &queue, &ids("a")).await;
    let mut per_track_failed = 0;
    for id in ids("b") {
        let mut transaction = pool.begin().await?;
        match super::super::wake::enqueue_in(&queue, &mut transaction, &id).await {
            Ok(()) => transaction.commit().await?,
            Err(_) => per_track_failed += 1,
        }
    }

    assert!(many.is_err());
    assert_eq!(per_track_failed, 1);
    let outcome = |prefix: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_as::<_, (String, bool, bool)>(
                "SELECT substr(state.sc_track_id, 2),
                        EXISTS (SELECT 1 FROM background_jobs AS job
                                WHERE job.kind = 'lyrics.lookup' AND job.dedup_key = state.sc_track_id),
                        state.wake_durable_at IS NOT NULL
                 FROM lyrics_lookup_state AS state
                 WHERE state.sc_track_id LIKE $1 || '%'
                 ORDER BY 1",
            )
            .bind(prefix)
            .fetch_all(&pool)
            .await
        }
    };
    let (many_rows, per_track_rows) = (outcome("a").await?, outcome("b").await?);
    assert_eq!(many_rows, per_track_rows);
    assert_eq!(many_rows.iter().filter(|row| row.1).count(), 4);
    Ok(())
}

fn offline_handler(pool: &PgPool) -> anyhow::Result<super::LyricsLookupHandler> {
    let http = sc_fingerprint::builder(None)
        .timeout(Duration::from_secs(1))
        .build()?;
    let fetcher = ExternalFetcher::new(http, String::new(), None);
    let genius = GeniusService::new(
        fetcher.clone(),
        GeniusCfg {
            access_token: String::new(),
            max_concurrent_scrapes: 8,
        },
    );
    let sources = LyricsSources::new(
        fetcher,
        genius,
        "https://apic-desktop.musixmatch.com/ws/1.1".to_owned(),
    );
    Ok(super::LyricsLookupHandler::new(
        pool.clone(),
        sources,
        LyricsConfig {
            batch: 8,
            concurrency: 4,
            claim_seconds: 900,
            backfill_batch: 100,
            musixmatch_base: "https://apic-desktop.musixmatch.com/ws/1.1".to_owned(),
        },
    ))
}

async fn lookup_job(pool: &PgPool) -> anyhow::Result<LeasedJob> {
    let queue = JobRepository::new(pool.clone(), "lyrics-recheck".to_owned());
    super::super::wake::enqueue(pool, &queue, "42").await?;
    queue
        .claim(
            &[backend_contracts::JobKind::LyricsLookup],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("lookup job was not claimed"))
}

async fn claim_track(
    pool: &PgPool,
    handler: &super::LyricsLookupHandler,
    job: &LeasedJob,
) -> anyhow::Result<super::LookupClaim> {
    sqlx::query("UPDATE lyrics_lookup_state SET next_run_at = now() WHERE sc_track_id = '42'")
        .execute(pool)
        .await?;
    claim_now(pool, job, handler.config.claim_seconds)
        .await?
        .ok_or_else(|| anyhow::anyhow!("track was not claimed"))
}

async fn claim_now(
    pool: &PgPool,
    job: &LeasedJob,
    claim_seconds: i64,
) -> anyhow::Result<Option<super::LookupClaim>> {
    Ok(sqlx::query_file_as!(
        super::LookupClaim,
        "queries/lyrics/lookup_claim_target.sql",
        job.id,
        job.generation,
        job.lease_id,
        "42",
        claim_seconds
    )
    .fetch_optional(pool)
    .await?)
}

fn candidate(source: &str, synced: Option<&str>, plain: &str) -> LyricsCandidate {
    LyricsCandidate {
        source: source.to_owned(),
        synced_lrc: synced.map(str::to_owned),
        plain_text: Some(plain.to_owned()),
        artist: None,
        title: None,
        duration_sec: None,
        exact: true,
        query_index: 0,
    }
}

const PLAIN_BODY: &str = "A sufficiently long plain lyrics body from genius";

async fn settle(
    pool: &PgPool,
    handler: &super::LyricsLookupHandler,
    job: &LeasedJob,
    candidate: LyricsCandidate,
    recheck: bool,
) -> anyhow::Result<()> {
    let claim = claim_track(pool, handler, job).await?;
    handler
        .settle_found(job, &claim, candidate, recheck)
        .await?;
    Ok(())
}

async fn recheck_state(pool: &PgPool) -> anyhow::Result<Option<(String, i32, bool, bool, bool)>> {
    Ok(sqlx::query_as(
        "SELECT state.status,
                state.failure_streak,
                state.next_run_at BETWEEN now() + interval '23 hours' AND now() + interval '25 hours'
                    AND queue.next_run_at = state.next_run_at,
                queue.claim_job_id IS NULL,
                state.wake_message_id IS NULL AND queue.wake_at IS NULL
         FROM lyrics_lookup_state AS state
         JOIN lyrics_lookup_queue AS queue ON queue.track_id = state.track_id
         WHERE state.sc_track_id = '42'",
    )
    .fetch_optional(pool)
    .await?)
}

async fn cached_text(pool: &PgPool) -> anyhow::Result<(Option<String>, Option<String>, i64)> {
    Ok(sqlx::query_as(
        "SELECT synced_lrc, plain_text, content_generation FROM lyrics_cache WHERE sc_track_id = '42'",
    )
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn plain_text_found_while_lrclib_failed_keeps_the_lookup_for_a_day(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;

    settle(
        &pool,
        &handler,
        &job,
        candidate("genius", None, PLAIN_BODY),
        true,
    )
    .await?;

    let (synced, plain, _) = cached_text(&pool).await?;
    assert_eq!(synced, None);
    assert_eq!(plain.as_deref(), Some(PLAIN_BODY));
    assert_eq!(
        recheck_state(&pool).await?,
        Some(("retry".to_owned(), 1, true, true, true))
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn plain_text_found_with_lrclib_healthy_ends_the_lookup(pool: PgPool) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;

    settle(
        &pool,
        &handler,
        &job,
        candidate("genius", None, PLAIN_BODY),
        false,
    )
    .await?;

    assert_eq!(cached_text(&pool).await?.1.as_deref(), Some(PLAIN_BODY));
    assert_eq!(recheck_state(&pool).await?, None);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_later_lookup_with_synced_text_fills_the_plain_only_row(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;
    settle(
        &pool,
        &handler,
        &job,
        candidate("genius", None, PLAIN_BODY),
        true,
    )
    .await?;

    let synced = candidate("lrclib", Some("[00:01.00] A synced line"), "A synced line");
    settle(&pool, &handler, &job, synced, false).await?;

    let (synced_lrc, plain, _) = cached_text(&pool).await?;
    assert_eq!(synced_lrc.as_deref(), Some("[00:01.00] A synced line"));
    assert_eq!(plain.as_deref(), Some(PLAIN_BODY));
    assert_eq!(
        sync_provenance(&pool).await?,
        (Some("lrclib".to_owned()), None)
    );
    assert_eq!(recheck_state(&pool).await?, None);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_recheck_stops_after_a_week_and_repeats_change_nothing(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;
    sqlx::query("UPDATE lyrics_lookup_state SET failure_streak = 5 WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;

    for streak in [6, 7] {
        settle(
            &pool,
            &handler,
            &job,
            candidate("genius", None, PLAIN_BODY),
            true,
        )
        .await?;
        assert_eq!(
            recheck_state(&pool).await?,
            Some(("retry".to_owned(), streak, true, true, true))
        );
    }
    assert_eq!(cached_text(&pool).await?.2, 1);

    settle(
        &pool,
        &handler,
        &job,
        candidate("genius", None, PLAIN_BODY),
        true,
    )
    .await?;

    assert_eq!(recheck_state(&pool).await?, None);
    assert_eq!(cached_text(&pool).await?.1.as_deref(), Some(PLAIN_BODY));
    Ok(())
}

const OUTAGE_BACKFILL: &str = "
SET lock_timeout = '5s';
INSERT INTO lyrics_lookup_state (
    track_id, sc_track_id, status, priority, next_run_at,
    input_title, input_artist, input_duration_ms, input_genius_song_id,
    input_genius_url, input_release_date, input_sc_created_at,
    failure_streak, created_at
)
SELECT track.id, track.sc_track_id, 'retry', track.index_priority,
       now() + random() * interval '24 hours',
       btrim(track.title),
       COALESCE(NULLIF(btrim(track.metadata_artist), ''), NULLIF(btrim(track.uploader_username), ''), ''),
       track.duration_ms, track.genius_song_id, NULLIF(btrim(track.genius_url), ''),
       track.release_date, track.sc_created_at, 1, track.created_at
FROM lyrics_cache AS cache
JOIN tracks AS track ON track.sc_track_id = cache.sc_track_id
WHERE cache.updated_at >= '2026-10-31 04:00+00'
  AND NULLIF(btrim(cache.synced_lrc), '') IS NULL
  AND NULLIF(btrim(cache.plain_text), '') IS NOT NULL
  AND cache.plain_source IN ('genius', 'musixmatch', 'netease')
  AND NOT track.pipeline_held
  AND NOT EXISTS (SELECT 1 FROM lyrics_lookup_state AS state WHERE state.track_id = track.id)
LIMIT 5000
ON CONFLICT (track_id) DO NOTHING;
";

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_outage_backfill_reschedules_only_plain_only_rows_without_a_state(
    pool: PgPool,
) -> anyhow::Result<()> {
    for id in ["1", "2", "3", "4", "5"] {
        insert_track(&pool, id, "Lose Yourself").await?;
    }
    for (id, synced, source, updated) in [
        ("1", None, "genius", "2026-10-31 06:00+00"),
        (
            "2",
            Some("[00:01.00] line"),
            "genius",
            "2026-10-31 06:00+00",
        ),
        ("3", None, "musixmatch", "2026-10-31 06:00+00"),
        ("4", None, "genius", "2026-10-30 06:00+00"),
        ("5", None, "lrclib", "2026-10-31 06:00+00"),
    ] {
        sqlx::query(
            "INSERT INTO lyrics_cache (sc_track_id, synced_lrc, plain_text, source, plain_source, updated_at)
             VALUES ($1, $2, 'plain', $3, $3, $4::timestamptz)",
        )
        .bind(id)
        .bind(synced)
        .bind(source)
        .bind(updated)
        .execute(&pool)
        .await?;
    }
    sqlx::query("DELETE FROM lyrics_lookup_state WHERE sc_track_id <> '3'")
        .execute(&pool)
        .await?;

    for _ in 0..2 {
        sqlx::raw_sql(OUTAGE_BACKFILL).execute(&pool).await?;
    }

    let rows: Vec<(String, String, i32, bool)> = sqlx::query_as(
        "SELECT sc_track_id, status, failure_streak,
                next_run_at <= now() + interval '24 hours'
         FROM lyrics_lookup_state ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        rows,
        vec![
            ("1".to_owned(), "retry".to_owned(), 1, true),
            ("3".to_owned(), "pending".to_owned(), 0, true),
        ]
    );
    Ok(())
}

#[derive(Debug, PartialEq, Eq, sqlx::FromRow)]
struct Queued {
    claim_job_id: Option<uuid::Uuid>,
    attempts: i64,
    priority: i16,
    due: bool,
    wake_open: bool,
}

async fn queued(pool: &PgPool) -> anyhow::Result<Option<Queued>> {
    Ok(sqlx::query_as(
        "SELECT queue.claim_job_id,
                queue.attempts,
                queue.priority,
                queue.next_run_at <= now() AS due,
                queue.wake_at IS NOT NULL AS wake_open
         FROM lyrics_lookup_queue AS queue
         JOIN lyrics_lookup_state AS state ON state.track_id = queue.track_id
         WHERE state.sc_track_id = '42'",
    )
    .fetch_optional(pool)
    .await?)
}

async fn version(pool: &PgPool) -> anyhow::Result<Option<(String, i64, String, i32)>> {
    Ok(sqlx::query_as(
        "SELECT ctid::text || ':' || xmin::text, generation, status, failure_streak
         FROM lyrics_lookup_state WHERE sc_track_id = '42'",
    )
    .fetch_optional(pool)
    .await?)
}

async fn writes(connection: &mut sqlx::PgConnection) -> anyhow::Result<(i64, i64)> {
    Ok(sqlx::query_as(
        "SELECT COALESCE(sum(n_tup_upd) FILTER (WHERE relname = 'lyrics_lookup_state'), 0)::bigint,
                COALESCE(sum(n_tup_upd) FILTER (WHERE relname = 'lyrics_lookup_queue'), 0)::bigint
         FROM pg_stat_xact_user_tables",
    )
    .fetch_one(&mut *connection)
    .await?)
}

async fn since(
    connection: &mut sqlx::PgConnection,
    base: (i64, i64),
) -> anyhow::Result<(i64, i64)> {
    let now = writes(connection).await?;
    Ok((now.0 - base.0, now.1 - base.1))
}

async fn settle_miss_in(
    connection: &mut sqlx::PgConnection,
    job: &LeasedJob,
    claim: &super::LookupClaim,
) -> anyhow::Result<Option<uuid::Uuid>> {
    Ok(sqlx::query_file_scalar!(
        "queries/lyrics/lookup_settle_miss.sql",
        job.id,
        job.generation,
        job.lease_id,
        claim.track_id,
        claim.generation,
        0_i64,
        "not_found"
    )
    .fetch_optional(&mut *connection)
    .await?)
}

async fn settle_retry_in(
    connection: &mut sqlx::PgConnection,
    job: &LeasedJob,
    claim: &super::LookupClaim,
) -> anyhow::Result<Option<uuid::Uuid>> {
    Ok(sqlx::query_file_scalar!(
        "queries/lyrics/lookup_settle_retry.sql",
        job.id,
        job.generation,
        job.lease_id,
        claim.track_id,
        claim.generation,
        0_i64,
        "transport",
        "transport",
        0_i64
    )
    .fetch_optional(&mut *connection)
    .await?)
}

async fn settle_found_in(
    connection: &mut sqlx::PgConnection,
    job: &LeasedJob,
    claim: &super::LookupClaim,
    synced: &str,
    recheck: bool,
) -> anyhow::Result<Option<bool>> {
    Ok(sqlx::query_file!(
        "queries/lyrics/lookup_settle_found.sql",
        job.id,
        job.generation,
        job.lease_id,
        claim.track_id,
        claim.generation,
        "genius",
        synced,
        PLAIN_BODY,
        recheck,
        super::RECHECK_MAX_STREAK
    )
    .fetch_optional(&mut *connection)
    .await?
    .map(|row| row.settled))
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_claim_writes_only_the_queue_and_each_settle_writes_the_state_once(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let job = lookup_job(&pool).await?;

    for round in 0..3_i64 {
        let before = version(&pool).await?;
        let mut transaction = pool.begin().await?;
        let base = writes(&mut transaction).await?;
        let claim = sqlx::query_file_as!(
            super::LookupClaim,
            "queries/lyrics/lookup_claim_target.sql",
            job.id,
            job.generation,
            job.lease_id,
            "42",
            900_i64
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| anyhow::anyhow!("track was not claimed"))?;
        assert_eq!(since(&mut transaction, base).await?, (0, 1));
        transaction.commit().await?;
        assert_eq!(version(&pool).await?, before);
        assert_eq!(
            queued(&pool)
                .await?
                .map(|row| (row.claim_job_id, row.attempts)),
            Some((Some(job.id), round + 1))
        );

        let mut transaction = pool.begin().await?;
        let base = writes(&mut transaction).await?;
        let settled = match round {
            0 => settle_miss_in(&mut transaction, &job, &claim)
                .await?
                .is_some(),
            1 => settle_retry_in(&mut transaction, &job, &claim)
                .await?
                .is_some(),
            _ => settle_found_in(&mut transaction, &job, &claim, "", true).await? == Some(true),
        };
        assert!(settled, "round {round} did not settle");
        assert_eq!(since(&mut transaction, base).await?, (1, 1));
        transaction.commit().await?;
        let row = queued(&pool)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no queue row"))?;
        assert_eq!(row.claim_job_id, None);
        assert!(!row.wake_open);
        sqlx::query("UPDATE lyrics_lookup_state SET next_run_at = now() WHERE sc_track_id = '42'")
            .execute(&pool)
            .await?;
    }
    let status: (String, String) = sqlx::query_as(
        "SELECT status, last_outcome FROM lyrics_lookup_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(status, ("retry".to_owned(), "recheck".to_owned()));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_found_synced_result_deletes_the_state_and_the_queue_row_in_one_statement(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;
    let claim = claim_track(&pool, &handler, &job).await?;

    let mut transaction = pool.begin().await?;
    let base = writes(&mut transaction).await?;
    let settled = settle_found_in(
        &mut transaction,
        &job,
        &claim,
        "[00:01.00] A synced line",
        false,
    )
    .await?;
    assert_eq!(settled, Some(true));
    assert_eq!(since(&mut transaction, base).await?, (0, 0));
    transaction.commit().await?;

    assert_eq!(version(&pool).await?, None);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_lookup_queue")
        .fetch_one(&pool)
        .await?;
    assert_eq!(left, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_stale_claim_is_released_without_a_state_write(pool: PgPool) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;

    for kind in ["miss", "retry", "found"] {
        let claim = claim_track(&pool, &handler, &job).await?;
        sqlx::raw_sql(
            "ALTER TABLE lyrics_lookup_state DISABLE TRIGGER lyrics_lookup_queue_sync;
             UPDATE lyrics_lookup_state SET generation = generation + 1 WHERE sc_track_id = '42';
             ALTER TABLE lyrics_lookup_state ENABLE ALWAYS TRIGGER lyrics_lookup_queue_sync;",
        )
        .execute(&pool)
        .await?;
        let before = version(&pool).await?;

        let mut transaction = pool.begin().await?;
        let base = writes(&mut transaction).await?;
        let answered = match kind {
            "miss" => settle_miss_in(&mut transaction, &job, &claim)
                .await?
                .is_some(),
            "retry" => settle_retry_in(&mut transaction, &job, &claim)
                .await?
                .is_some(),
            _ => settle_found_in(&mut transaction, &job, &claim, "[00:01.00] line", false)
                .await?
                .is_some(),
        };
        assert!(!answered, "a stale {kind} settle answered");
        assert_eq!(since(&mut transaction, base).await?, (0, 1));
        transaction.commit().await?;

        assert_eq!(version(&pool).await?, before);
        let row = queued(&pool)
            .await?
            .ok_or_else(|| anyhow::anyhow!("no queue row"))?;
        assert_eq!((row.claim_job_id, row.due), (None, true));
    }
    let cached: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_cache")
        .fetch_one(&pool)
        .await?;
    assert_eq!(cached, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_input_change_during_a_claim_clears_it_and_fences_out_its_settle(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;
    let claim = claim_track(&pool, &handler, &job).await?;

    sqlx::query("UPDATE tracks SET title = 'Lose Yourself (Live)' WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;
    let row = queued(&pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no queue row"))?;
    assert_eq!(
        (row.claim_job_id, row.due, row.wake_open),
        (None, true, true)
    );
    let before = version(&pool).await?;

    let mut connection = pool.acquire().await?;
    let settled = settle_found_in(&mut connection, &job, &claim, "[00:01.00] line", false).await?;
    assert_eq!(settled, None);
    assert_eq!(version(&pool).await?, before);

    let reclaimed = claim_now(&pool, &job, 900)
        .await?
        .ok_or_else(|| anyhow::anyhow!("track was not claimed again"))?;
    assert_eq!(reclaimed.generation, claim.generation + 1);
    assert_eq!(reclaimed.input_title, "Lose Yourself (Live)");
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_wake_during_a_claim_reopens_the_lookup_and_fences_out_the_old_settle(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;
    let first = claim_track(&pool, &handler, &job).await?;
    let mut connection = pool.acquire().await?;
    assert!(
        settle_miss_in(&mut connection, &job, &first)
            .await?
            .is_some()
    );
    let claim = claim_now(&pool, &job, 900)
        .await?
        .ok_or_else(|| anyhow::anyhow!("due miss was not claimed"))?;
    let due: Vec<String> = sqlx::query_file_scalar!("queries/lyrics/lookup_due_wakes.sql", 10_i64)
        .fetch_all(&pool)
        .await?;
    assert!(due.is_empty());

    let answer: serde_json::Value = sqlx::query_scalar(
        "SELECT public.scd_cmd_lyrics_wake(jsonb_build_object('sc_track_id', '42', 'at', now())) -> 'output'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(answer["status"], "pending");
    assert!(answer.get("claim_job_id").is_none());
    let row = queued(&pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no queue row"))?;
    assert_eq!(
        (row.claim_job_id, row.priority, row.due, row.wake_open),
        (None, 0, true, true)
    );

    assert!(
        settle_retry_in(&mut connection, &job, &claim)
            .await?
            .is_none()
    );
    let state = version(&pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no state"))?;
    assert_eq!(
        (state.1, state.2.as_str()),
        (claim.generation + 1, "pending")
    );
    let due: Vec<String> = sqlx::query_file_scalar!("queries/lyrics/lookup_due_wakes.sql", 10_i64)
        .fetch_all(&pool)
        .await?;
    assert_eq!(due, ["42"]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_expired_claim_is_released_on_the_queue_only(pool: PgPool) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let job = lookup_job(&pool).await?;
    let claimed = claim_now(&pool, &job, 0).await?;
    assert!(claimed.is_some());
    let before = version(&pool).await?;

    let released = sqlx::query_file!("queries/lyrics/lookup_release_expired.sql", 10_i64)
        .execute(&pool)
        .await?
        .rows_affected();

    assert_eq!(released, 1);
    assert_eq!(version(&pool).await?, before);
    let row = queued(&pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no queue row"))?;
    assert_eq!((row.claim_job_id, row.attempts, row.due), (None, 1, true));
    assert!(claim_now(&pool, &job, 900).await?.is_some());
    Ok(())
}

async fn observe_and_bump(pool: &PgPool, priority: i16) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE tracks
         SET title = title, uploader_username = uploader_username, duration_ms = duration_ms
         WHERE sc_track_id = '42'",
    )
    .execute(pool)
    .await?;
    sqlx::query("UPDATE tracks SET index_priority = $1 WHERE sc_track_id = '42'")
        .bind(priority)
        .execute(pool)
        .await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_recheck_row_survives_an_observe_and_a_priority_bump_and_ends_with_synced_lyrics(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;
    settle(
        &pool,
        &handler,
        &job,
        candidate("genius", None, PLAIN_BODY),
        true,
    )
    .await?;
    let rescheduled = version(&pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("recheck row is gone"))?;

    sqlx::query(
        "UPDATE tracks
         SET title = title, uploader_username = uploader_username, duration_ms = duration_ms
         WHERE sc_track_id = '42'",
    )
    .execute(&pool)
    .await?;
    assert_eq!(version(&pool).await?.as_ref(), Some(&rescheduled));

    sqlx::query("UPDATE tracks SET index_priority = 1 WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;
    let bumped = version(&pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("recheck row is gone"))?;
    assert_eq!(
        (bumped.1, bumped.2.as_str(), bumped.3),
        (rescheduled.1, "retry", 1)
    );
    assert_eq!(
        recheck_state(&pool).await?,
        Some(("retry".to_owned(), 1, true, true, true))
    );
    let row = queued(&pool)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no queue row"))?;
    assert_eq!((row.priority, row.due), (1, false));

    sqlx::query("UPDATE tracks SET title = 'Lose Yourself (Live)' WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;
    let renamed: (i64, String, i32, String, bool) = sqlx::query_as(
        "SELECT generation, status, failure_streak, input_title, wake_message_id IS NULL
         FROM lyrics_lookup_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        renamed,
        (
            rescheduled.1 + 1,
            "retry".to_owned(),
            1,
            "Lose Yourself (Live)".to_owned(),
            true
        )
    );

    let synced = candidate("lrclib", Some("[00:01.00] A synced line"), "A synced line");
    settle(&pool, &handler, &job, synced, false).await?;
    assert_eq!(version(&pool).await?, None);
    assert_eq!(
        cached_text(&pool).await?.0.as_deref(),
        Some("[00:01.00] A synced line")
    );
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_lookup_queue")
        .fetch_one(&pool)
        .await?;
    assert_eq!(left, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_recheck_row_survives_observes_and_ends_after_seven_tries(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;

    for streak in 1..=super::RECHECK_MAX_STREAK {
        settle(
            &pool,
            &handler,
            &job,
            candidate("genius", None, PLAIN_BODY),
            true,
        )
        .await?;
        observe_and_bump(&pool, i16::try_from(streak % 5)?).await?;
        assert_eq!(
            recheck_state(&pool).await?,
            Some(("retry".to_owned(), streak, true, true, true)),
            "recheck {streak} lost its row"
        );
    }

    settle(
        &pool,
        &handler,
        &job,
        candidate("genius", None, PLAIN_BODY),
        true,
    )
    .await?;
    assert_eq!(version(&pool).await?, None);
    observe_and_bump(&pool, 0).await?;
    sqlx::query("UPDATE tracks SET title = 'Lose Yourself (Live)' WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;
    assert_eq!(version(&pool).await?, None);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_lookup_queue")
        .fetch_one(&pool)
        .await?;
    assert_eq!(left, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn external_synced_lyrics_end_a_recheck_row_on_the_next_track_update(
    pool: PgPool,
) -> anyhow::Result<()> {
    insert_track(&pool, "42", "Lose Yourself").await?;
    let handler = offline_handler(&pool)?;
    let job = lookup_job(&pool).await?;
    settle(
        &pool,
        &handler,
        &job,
        candidate("genius", None, PLAIN_BODY),
        true,
    )
    .await?;
    sqlx::query(
        "UPDATE lyrics_cache
         SET synced_lrc = '[00:01.00] A synced line', synced_source = 'lrclib', source = 'lrclib'
         WHERE sc_track_id = '42'",
    )
    .execute(&pool)
    .await?;
    assert!(version(&pool).await?.is_some());

    observe_and_bump(&pool, 3).await?;

    assert_eq!(version(&pool).await?, None);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_lookup_queue")
        .fetch_one(&pool)
        .await?;
    assert_eq!(left, 0);
    Ok(())
}

type WakeOutcome = (
    String,
    Option<bool>,
    Option<i16>,
    Option<String>,
    Option<i16>,
    Option<String>,
    bool,
);

async fn wake_outcome(pool: &PgPool, prefix: &str) -> anyhow::Result<Vec<WakeOutcome>> {
    Ok(sqlx::query_as::<_, WakeOutcome>(
        "SELECT substr(state.sc_track_id, 2),
                job.id = state.wake_message_id,
                job.priority,
                job.lane,
                job.max_attempts,
                replace(job.payload::text, state.sc_track_id, 'track'),
                state.wake_durable_at IS NOT NULL
         FROM lyrics_lookup_state AS state
         LEFT JOIN background_jobs AS job
           ON job.kind = 'lyrics.lookup' AND job.dedup_key = state.sc_track_id
         WHERE state.sc_track_id LIKE $1 || '%'
         ORDER BY 1",
    )
    .bind(prefix)
    .fetch_all(pool)
    .await?)
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_batched_wake_leaves_the_rows_of_one_wake_per_track(pool: PgPool) -> anyhow::Result<()> {
    let queue = JobRepository::new(pool.clone(), "wake-batch".to_owned());
    for prefix in ["a", "b", "c"] {
        for number in 1..=6 {
            insert_track(&pool, &format!("{prefix}{number}"), "Track").await?;
        }
        sqlx::raw_sql(&format!(
            "UPDATE lyrics_lookup_state SET wake_durable_at = now() WHERE sc_track_id = '{prefix}2';
             UPDATE lyrics_lookup_state SET generation = generation + 1 WHERE sc_track_id = '{prefix}3';
             INSERT INTO background_job_enqueues (id)
             SELECT wake_message_id FROM lyrics_lookup_state WHERE sc_track_id = '{prefix}5';"
        ))
        .execute(&pool)
        .await?;
        queue
            .enqueue(&crate::queue::NewJob {
                id: uuid::Uuid::now_v7(),
                kind: backend_contracts::JobKind::LyricsLookup,
                dedup_key: Some(format!("{prefix}4")),
                payload: serde_json::json!({}),
                priority: 1,
                max_attempts: 3,
                available_at: chrono::Utc::now(),
            })
            .await?;
    }
    let ids = |prefix: &str| {
        ["6", "1", "2", "3", "4", "5", "1", "missing"]
            .map(|number| format!("{prefix}{number}"))
            .to_vec()
    };

    super::super::wake::enqueue_many(&pool, &queue, &ids("a")).await?;
    for id in ids("b") {
        let mut transaction = pool.begin().await?;
        super::super::wake::enqueue_in(&queue, &mut transaction, &id).await?;
        transaction.commit().await?;
    }
    let mut transaction = pool.begin().await?;
    super::super::wake::enqueue_all_in(&mut transaction, &ids("c")).await?;
    transaction.commit().await?;

    let per_track = wake_outcome(&pool, "b").await?;
    assert_eq!(wake_outcome(&pool, "a").await?, per_track);
    assert_eq!(wake_outcome(&pool, "c").await?, per_track);
    assert_eq!(
        per_track
            .iter()
            .map(|row| (row.0.as_str(), row.1, row.2, row.6))
            .collect::<Vec<_>>(),
        [
            ("1", Some(true), Some(10), true),
            ("2", None, None, true),
            ("3", None, None, false),
            ("4", Some(false), Some(10), true),
            ("5", None, None, true),
            ("6", Some(true), Some(10), true),
        ]
    );
    Ok(())
}
