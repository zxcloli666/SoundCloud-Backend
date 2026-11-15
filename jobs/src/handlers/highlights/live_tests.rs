use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use backend_contracts::pipeline::{HIGHLIGHT_VERSION, HighlightResult, MomentHints, Producer};
use backend_contracts::reasons::{WorkerReason, WorkerStatus};
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use url::Url;
use uuid::Uuid;

use crate::bus::Bus;
use crate::config::{HighlightConfig, NatsConfig};

use super::HighlightHandler;
use super::queue::{self, Claimed};
use super::result;

const TIMEOUT: Duration = Duration::from_secs(1_800);
const MAX_FAILURES: u32 = 3;

async fn track(pool: &PgPool, sc_track_id: &str, plays: i64, playable: bool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, sharing, duration_ms,
                             storage_state, index_state, play_count_sc)
         VALUES ($1, 'soundcloud:tracks:' || $1, 'Song', 'song', 'public', 200000,
                 'ok', CASE WHEN $2 THEN 'indexed' ELSE 'pending' END, $3)",
    )
    .bind(sc_track_id)
    .bind(playable)
    .bind(plays)
    .execute(pool)
    .await?;
    Ok(())
}

async fn dispatch_state(pool: &PgPool, sc_track_id: &str) -> anyhow::Result<(String, i32, i16)> {
    Ok(sqlx::query_as(
        "SELECT dispatch.state, dispatch.attempt, dispatch.failures
         FROM track_highlight_dispatch AS dispatch
         JOIN tracks AS track ON track.id = dispatch.track_id
         WHERE track.sc_track_id = $1",
    )
    .bind(sc_track_id)
    .fetch_one(pool)
    .await?)
}

async fn stored(pool: &PgPool, sc_track_id: &str) -> anyhow::Result<Option<(i32, i32, f32, i16)>> {
    Ok(sqlx::query_as(
        "SELECT highlight.start_ms, highlight.length_ms, highlight.score, highlight.version
         FROM track_highlights AS highlight
         JOIN tracks AS track ON track.id = highlight.track_id
         WHERE track.sc_track_id = $1",
    )
    .bind(sc_track_id)
    .fetch_optional(pool)
    .await?)
}

fn done(sc_track_id: &str, attempt: i64, reason: Option<WorkerReason>) -> HighlightResult {
    let found = reason.is_none();
    HighlightResult {
        sc_track_id: sc_track_id.to_owned(),
        attempt,
        status: reason.map_or(WorkerStatus::Ok, WorkerReason::status),
        reason,
        detail: None,
        producer: Producer {
            worker_id: "sc-worker1".to_owned(),
            build: "test".to_owned(),
            models: BTreeMap::new(),
            sync_version: None,
        },
        version: found.then_some(HIGHLIGHT_VERSION),
        start_ms: found.then_some(52_500),
        length_ms: found.then_some(30_000),
        score: found.then_some(0.75),
    }
}

fn ids(claimed: &[Claimed]) -> Vec<&str> {
    let mut ids: Vec<&str> = claimed
        .iter()
        .map(|claim| claim.sc_track_id.as_str())
        .collect();
    ids.sort_unstable();
    ids
}

async fn record(pool: &PgPool, result: &HighlightResult) -> anyhow::Result<()> {
    result::record(pool, MAX_FAILURES, result)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

async fn claim(pool: &PgPool, in_flight: i64) -> anyhow::Result<Vec<Claimed>> {
    queue::claim(pool, in_flight, TIMEOUT)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

async fn walk(pool: &PgPool, after: Uuid, batch: i64) -> anyhow::Result<Uuid> {
    queue::walk(pool, after, batch)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_most_popular_playable_tracks_go_first_and_in_flight_stays_bounded(
    pool: PgPool,
) -> anyhow::Result<()> {
    track(&pool, "11", 10, true).await?;
    track(&pool, "12", 5_000, true).await?;
    track(&pool, "13", 900, true).await?;
    track(&pool, "14", 9_000, false).await?;
    track(&pool, "15", 8_000, true).await?;
    sqlx::query("UPDATE tracks SET sharing = 'private' WHERE sc_track_id = '15'")
        .execute(&pool)
        .await?;

    assert_eq!(walk(&pool, Uuid::nil(), 100).await?, Uuid::nil());
    let first = claim(&pool, 2).await?;
    let second = claim(&pool, 2).await?;

    assert_eq!(ids(&first), ["12", "13"]);
    assert!(
        first
            .iter()
            .all(|claim| claim.attempt == 1 && claim.version == HIGHLIGHT_VERSION)
    );
    assert_eq!(
        first[0].message_id(),
        format!("highlight:{}:1", first[0].sc_track_id)
    );
    assert!(second.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_found_moment_is_stored_once_and_the_track_leaves_the_queue(
    pool: PgPool,
) -> anyhow::Result<()> {
    track(&pool, "21", 100, true).await?;
    walk(&pool, Uuid::nil(), 100).await?;
    claim(&pool, 4).await?;

    record(&pool, &done("21", 1, None)).await?;
    record(&pool, &done("21", 1, None)).await?;
    walk(&pool, Uuid::nil(), 100).await?;

    assert_eq!(
        stored(&pool, "21").await?,
        Some((52_500, 30_000, 0.75, HIGHLIGHT_VERSION))
    );
    assert_eq!(
        dispatch_state(&pool, "21").await?,
        ("done".to_owned(), 1, 0)
    );
    assert!(claim(&pool, 4).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_transient_failure_backs_off_and_settles_after_the_failure_cap(
    pool: PgPool,
) -> anyhow::Result<()> {
    track(&pool, "31", 100, true).await?;
    walk(&pool, Uuid::nil(), 100).await?;
    claim(&pool, 4).await?;

    record(&pool, &done("31", 1, Some(WorkerReason::DownloadFailed))).await?;

    assert_eq!(
        dispatch_state(&pool, "31").await?,
        ("queued".to_owned(), 1, 1)
    );
    assert!(claim(&pool, 4).await?.is_empty());
    for attempt in 2..=3 {
        sqlx::query("UPDATE track_highlight_dispatch SET next_run_at = now()")
            .execute(&pool)
            .await?;
        let retried = claim(&pool, 4).await?;
        assert_eq!(retried.first().map(|claim| claim.attempt), Some(attempt));
        record(
            &pool,
            &done(
                "31",
                i64::from(attempt),
                Some(WorkerReason::DeadlineExceeded),
            ),
        )
        .await?;
    }
    assert_eq!(
        dispatch_state(&pool, "31").await?,
        ("failed".to_owned(), 3, 3)
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_permanent_outcome_settles_and_a_stale_attempt_changes_nothing(
    pool: PgPool,
) -> anyhow::Result<()> {
    track(&pool, "41", 100, true).await?;
    walk(&pool, Uuid::nil(), 100).await?;
    claim(&pool, 4).await?;

    record(&pool, &done("41", 7, Some(WorkerReason::DownloadFailed))).await?;
    assert_eq!(
        dispatch_state(&pool, "41").await?,
        ("dispatched".to_owned(), 1, 0)
    );
    record(&pool, &done("41", 1, Some(WorkerReason::SilentAudio))).await?;
    walk(&pool, Uuid::nil(), 100).await?;

    assert_eq!(
        dispatch_state(&pool, "41").await?,
        ("failed".to_owned(), 1, 0)
    );
    assert_eq!(stored(&pool, "41").await?, None);
    assert!(claim(&pool, 4).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_unanswered_request_goes_out_again_under_the_same_attempt(
    pool: PgPool,
) -> anyhow::Result<()> {
    track(&pool, "51", 100, true).await?;
    walk(&pool, Uuid::nil(), 100).await?;
    let first = claim(&pool, 4).await?;
    sqlx::query("UPDATE track_highlight_dispatch SET dispatched_at = now() - interval '1 hour'")
        .execute(&pool)
        .await?;

    let again = claim(&pool, 4).await?;

    assert_eq!(first, again);
    assert_eq!(
        dispatch_state(&pool, "51").await?,
        ("dispatched".to_owned(), 1, 0)
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_walk_moves_in_batches_and_starts_over_at_the_end(pool: PgPool) -> anyhow::Result<()> {
    for id in ["61", "62", "63"] {
        track(&pool, id, 1, true).await?;
    }
    let mut after = Uuid::nil();
    let mut steps = 0;
    loop {
        after = walk(&pool, after, 2).await?;
        steps += 1;
        if after.is_nil() {
            break;
        }
    }
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM track_highlight_dispatch")
        .fetch_one(&pool)
        .await?;

    assert_eq!((steps, queued), (2, 3));
    Ok(())
}

fn nats_config() -> NatsConfig {
    NatsConfig {
        url: std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_owned()),
        job_ingress_concurrency: 1,
        impression_concurrency: 1,
        retry_delay: Duration::from_millis(100),
        max_age: Duration::from_secs(60 * 60),
        job_stream_max_bytes: 16 * 1024 * 1024,
        impression_stream_max_bytes: 16 * 1024 * 1024,
        storage_event_sources: Vec::new(),
    }
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_switched_off_sweep_waits_for_shutdown_instead_of_exiting(
    pool: PgPool,
) -> anyhow::Result<()> {
    let bus = Bus::connect(&nats_config(), "highlight-off-test").await?;
    let config = HighlightConfig {
        in_flight: 0,
        walk_batch: 100,
        rewalk: Duration::from_secs(3_600),
        timeout: TIMEOUT,
        max_failures: MAX_FAILURES,
    };
    let handler = Arc::new(HighlightHandler::new(
        pool,
        bus,
        Url::parse("http://127.0.0.1:9")?,
        config,
    ));
    let cancellation = CancellationToken::new();
    let sweep = tokio::spawn(handler.sweep(cancellation.clone()));

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!sweep.is_finished());

    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(5), sweep).await???;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_moment_from_the_audio_index_settles_the_track_without_a_fetch(
    pool: PgPool,
) -> anyhow::Result<()> {
    track(&pool, "41", 100, true).await?;
    let indexed = |version: i16, start_ms: i32| {
        sqlx::query(include_str!("../../../queries/highlights/indexed.sql"))
            .bind("41")
            .bind(start_ms)
            .bind(30_000_i32)
            .bind(0.6_f32)
            .bind(version)
    };

    indexed(HIGHLIGHT_VERSION, 61_000).execute(&pool).await?;
    indexed(HIGHLIGHT_VERSION - 1, 5_000).execute(&pool).await?;
    walk(&pool, Uuid::nil(), 100).await?;

    assert_eq!(
        stored(&pool, "41").await?,
        Some((61_000, 30_000, 0.6, HIGHLIGHT_VERSION))
    );
    assert_eq!(dispatch_state(&pool, "41").await?.0, "done");
    assert!(claim(&pool, 4).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn hints_carry_chorus_returns_comments_and_shared_replay_landings(
    pool: PgPool,
) -> anyhow::Result<()> {
    track(&pool, "51", 100, true).await?;
    sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, synced_lrc, source)
         VALUES ('51', E'[00:10.00]First verse here\\n[00:30.00]Hold me now\\n[01:30.00]Hold me now', 'lrclib')",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO track_comments (id, sc_track_id, user_urn, body, track_position_ms)
         SELECT gen_random_uuid(), '51', 'soundcloud:users:' || g, 'x', 60000 + g * 100
         FROM generate_series(1, 4) AS g",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO track_seeks (id, sc_user_id, sc_track_id, position_ms, from_ms, source)
         SELECT gen_random_uuid(), 'u' || g, '51', 61000 + g * 10, 120000, 'player'
         FROM generate_series(1, 3) AS g
         UNION ALL
         SELECT gen_random_uuid(), 'm' || g, '51', 20000, 0, 'moment'
         FROM generate_series(1, 5) AS g
         UNION ALL
         SELECT gen_random_uuid(), 'solo', '51', 150000, 0, 'player'",
    )
    .execute(&pool)
    .await?;

    let hints = super::hints::load(&pool, &["51".to_owned(), "52".to_owned()])
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let found = hints.get("51").cloned().unwrap_or_default();

    assert_eq!(found.chorus_ms, [30_000, 90_000]);
    assert_eq!(found.comment_ms, [60_100, 60_200, 60_300, 60_400]);
    assert_eq!(found.replay_ms, [61_020]);
    assert_eq!(hints.get("52"), Some(&MomentHints::default()));
    Ok(())
}
