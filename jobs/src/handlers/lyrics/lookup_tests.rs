use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::any;
use base64::Engine as _;
use catalog_sources::{ExternalFetcher, GeniusCfg, GeniusService, LyricsSources};
use sqlx::PgPool;

use crate::config::LyricsConfig;
use crate::queue::{ClaimOrder, JobRepository};

async fn install_base(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE tracks (
            id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
            sc_track_id text NOT NULL UNIQUE,
            urn text NOT NULL UNIQUE,
            title text NOT NULL,
            metadata_artist text,
            uploader_username text,
            duration_ms integer NOT NULL,
            genius_song_id bigint,
            genius_url text,
            release_date date,
            sc_created_at timestamptz,
            index_priority smallint NOT NULL DEFAULT 5,
            play_count_sc bigint,
            storage_state varchar(16) NOT NULL DEFAULT 'pending',
            index_state varchar(16) NOT NULL DEFAULT 'pending',
            needs_duration_resolve boolean NOT NULL DEFAULT false,
            transcribe_state varchar(16),
            created_at timestamptz NOT NULL DEFAULT now(),
            updated_at timestamptz NOT NULL DEFAULT now()
        );
        CREATE TABLE lyrics_cache (
            sc_track_id text PRIMARY KEY,
            synced_lrc text,
            plain_text text,
            source varchar(16) NOT NULL,
            language varchar(8),
            language_confidence real,
            embedded_at timestamptz,
            embedding_state varchar(16),
            created_at timestamp NOT NULL DEFAULT now()
        );
        CREATE TABLE storage_event_state (
            sc_track_id text PRIMARY KEY REFERENCES tracks(sc_track_id) ON DELETE CASCADE,
            uploaded_generation bigint NOT NULL,
            updated_at timestamptz NOT NULL DEFAULT now()
        );
        CREATE TABLE transcription_wire_state (
            sc_track_id text PRIMARY KEY,
            status varchar(16) NOT NULL
        );
        CREATE TABLE lyrics_embedding_wire_state (
            sc_track_id text PRIMARY KEY,
            status varchar(16) NOT NULL
        );",
    )
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0057_background_jobs.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

async fn install_lookup(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0082_lyrics_lookup_state.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0114_lyrics_synced_version.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_track(pool: &PgPool, id: &str, title: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (
            sc_track_id, urn, title, uploader_username, duration_ms, index_priority
         ) VALUES ($1, 'soundcloud:tracks:' || $1, $2, 'artist', 180000, 5)",
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

#[sqlx::test(migrations = false)]
async fn new_track_creates_due_lookup_state(pool: PgPool) -> anyhow::Result<()> {
    install_base(&pool).await?;
    install_lookup(&pool).await?;

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

#[sqlx::test(migrations = false)]
async fn input_change_reopens_once_and_unrelated_change_does_not(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_base(&pool).await?;
    install_lookup(&pool).await?;
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

#[sqlx::test(migrations = false)]
async fn external_cache_prevents_lookup_state(pool: PgPool) -> anyhow::Result<()> {
    install_base(&pool).await?;
    sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, plain_text, source)
         VALUES ('42', 'A sufficiently long external lyrics body', 'genius')",
    )
    .execute(&pool)
    .await?;
    install_lookup(&pool).await?;

    insert_track(&pool, "42", "Track").await?;

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_lookup_state")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn keyset_backfill_is_bounded_and_restart_safe(pool: PgPool) -> anyhow::Result<()> {
    install_base(&pool).await?;
    insert_track(&pool, "41", "First").await?;
    insert_track(&pool, "42", "Second").await?;
    install_lookup(&pool).await?;

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

#[sqlx::test(migrations = false)]
async fn clean_source_empty_persists_negative_without_worker_jobs(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_base(&pool).await?;
    install_lookup(&pool).await?;
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

#[sqlx::test(migrations = false)]
async fn source_outage_persists_retry_without_worker_jobs(pool: PgPool) -> anyhow::Result<()> {
    install_base(&pool).await?;
    install_lookup(&pool).await?;
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
    install_base(pool).await?;
    install_lookup(pool).await?;
    insert_track(pool, "42", "Lose Yourself").await?;
    sqlx::query("UPDATE tracks SET storage_state = 'ok' WHERE sc_track_id = '42'")
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO storage_event_state (sc_track_id, uploaded_generation)
         VALUES ('42', 1)",
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

#[sqlx::test(migrations = false)]
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

#[sqlx::test(migrations = false)]
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

#[sqlx::test(migrations = false)]
async fn immediate_wake_uses_state_message_identity(pool: PgPool) -> anyhow::Result<()> {
    install_base(&pool).await?;
    install_lookup(&pool).await?;
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
