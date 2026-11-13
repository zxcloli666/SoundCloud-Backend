use std::time::Duration;

use backend_contracts::JobKind;
use catalog_sources::{
    ExternalFetcher, GeniusCfg, GeniusService, GeniusText, GeniusTexts, LyricsFailure,
    LyricsSources,
};
use sqlx::PgPool;

use super::refetch::RefetchClaim;
use super::{LyricsLookupHandler, Work};
use crate::config::LyricsConfig;
use crate::handlers::star_priority::{Tier, Turn};
use crate::queue::{ClaimOrder, JobRepository, LeasedJob, NewJob};

const OLD: &str = "12 Contributors\nline two\nline three";
const NEW: &str = "zebrafish opening\nline two\nline three";
const CREATED: &str = "2026-11-01 12:00:00";

pub(super) fn track_ids(works: &[Work]) -> Vec<String> {
    works
        .iter()
        .map(|work| match work {
            Work::Lookup(claim) => claim.sc_track_id.clone(),
            Work::Refetch(claim) => claim.sc_track_id.clone(),
        })
        .collect()
}

fn handler(pool: &PgPool) -> anyhow::Result<LyricsLookupHandler> {
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
    Ok(LyricsLookupHandler::new(
        pool.clone(),
        sources,
        LyricsConfig {
            batch: 8,
            concurrency: 4,
            claim_seconds: 900,
            backfill_batch: 100,
            refetch_share_percent: 100,
            musixmatch_base: "https://apic-desktop.musixmatch.com/ws/1.1".to_owned(),
        },
    ))
}

async fn sweep_job(pool: &PgPool) -> anyhow::Result<LeasedJob> {
    let queue = JobRepository::new(pool.clone(), "lyrics-refetch".to_owned());
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

async fn seed(pool: &PgPool, id: &str, title: &str, priority: i16) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO lyrics_cache (
             sc_track_id, plain_text, source, plain_source, created_at, embedded_at, embedding_state
         ) VALUES ($1, $2, 'genius', 'genius', $3::timestamp, now(), 'done')",
    )
    .bind(id)
    .bind(OLD)
    .bind(CREATED)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, uploader_username, duration_ms,
             index_priority, storage_state
         ) VALUES ($1, 'soundcloud:tracks:' || $1, $2, lower($2), 'artist', 180000, 5, 'ok')",
    )
    .bind(id)
    .bind(title)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO storage_event_state (
             sc_track_id, stream, stream_sequence, event_published_at, uploaded_generation
         ) VALUES ($1, 'storage', 1, now(), 1)",
    )
    .bind(id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO lyrics_repair (sc_track_id, kind, cache_created_at, priority)
         VALUES ($1, 'refetch', $2::timestamp, $3)",
    )
    .bind(id)
    .bind(CREATED)
    .bind(priority)
    .execute(pool)
    .await?;
    Ok(())
}

async fn claim_one(pool: &PgPool, job: &LeasedJob) -> anyhow::Result<RefetchClaim> {
    let works =
        super::tiered::claim(pool, job, 10, 100, 900, Turn::led_by(Tier::StarLiked)).await?;
    works
        .into_iter()
        .find_map(|work| match work {
            Work::Refetch(claim) => Some(claim),
            Work::Lookup(_) => None,
        })
        .ok_or_else(|| anyhow::anyhow!("no refetch was claimed"))
}

fn linked(text: &str) -> GeniusTexts {
    GeniusTexts::Found(vec![GeniusText {
        text: text.to_owned(),
        linked: true,
    }])
}

fn failure() -> LyricsFailure {
    LyricsFailure {
        class: "transport".to_owned(),
        retry_after_seconds: None,
    }
}

type Version = (String, i64, String);

async fn version(pool: &PgPool, id: &str) -> anyhow::Result<Version> {
    Ok(sqlx::query_as(
        "SELECT xmin::text, content_generation, updated_at::text
         FROM lyrics_cache WHERE sc_track_id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?)
}

type Ledger = (String, Option<String>, i32, bool);

async fn ledger(pool: &PgPool, id: &str) -> anyhow::Result<Ledger> {
    Ok(sqlx::query_as(
        "SELECT status, outcome, attempts, claim_job_id IS NULL
         FROM lyrics_repair WHERE sc_track_id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?)
}

async fn due_refetches(pool: &PgPool, job: &LeasedJob) -> anyhow::Result<usize> {
    let works =
        super::tiered::claim(pool, job, 10, 100, 900, Turn::led_by(Tier::StarLiked)).await?;
    Ok(works
        .iter()
        .filter(|work| matches!(work, Work::Refetch(_)))
        .count())
}

async fn kept(pool: &PgPool, texts: GeniusTexts, outcome: &str) -> anyhow::Result<()> {
    seed(pool, "42", "Track", 0).await?;
    let job = sweep_job(pool).await?;
    let handler = handler(pool)?;
    let claim = claim_one(pool, &job).await?;
    let before = version(pool, "42").await?;

    handler.settle_refetch(&job, &claim, texts).await?;

    assert_eq!(version(pool, "42").await?, before);
    assert_eq!(
        ledger(pool, "42").await?,
        ("done".to_owned(), Some(outcome.to_owned()), 1, true)
    );
    assert_eq!(due_refetches(pool, &job).await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_better_text_replaces_the_stored_one_and_resets_the_derived_state(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, "42", "Track", 0).await?;
    let job = sweep_job(&pool).await?;
    let handler = handler(&pool)?;
    let claim = claim_one(&pool, &job).await?;

    handler.settle_refetch(&job, &claim, linked(NEW)).await?;

    let cache: (
        String,
        i64,
        Option<String>,
        Option<String>,
        bool,
        String,
        String,
    ) = sqlx::query_as(
        "SELECT plain_text, content_generation, embedding_state, source,
                    embedded_at IS NULL, plain_source, created_at::text
             FROM lyrics_cache WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let found: bool = sqlx::query_scalar(
        "SELECT fts @@ plainto_tsquery('simple', 'zebrafish') FROM lyrics_cache WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let old: Option<String> =
        sqlx::query_scalar("SELECT old_plain_text FROM lyrics_repair WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    let downstream: Vec<String> = sqlx::query_scalar(
        "SELECT kind FROM background_jobs WHERE kind = 'lyrics.dispatch_transcription'",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(cache.0, NEW);
    assert_eq!(cache.1, 2);
    assert_eq!(cache.2, None);
    assert_eq!(cache.3.as_deref(), Some("genius"));
    assert!(cache.4);
    assert_eq!(cache.5, "genius");
    assert_eq!(cache.6, "2026-11-01 12:00:00");
    assert!(found);
    assert_eq!(old.as_deref(), Some(OLD));
    assert_eq!(
        ledger(&pool, "42").await?,
        ("done".to_owned(), Some("replaced".to_owned()), 1, true)
    );
    assert_eq!(downstream, vec!["lyrics.dispatch_transcription".to_owned()]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_different_text_keeps_the_stored_one(pool: PgPool) -> anyhow::Result<()> {
    kept(&pool, linked("a short page"), "different").await
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_same_text_is_unchanged(pool: PgPool) -> anyhow::Result<()> {
    kept(&pool, linked("line two\nline three"), "unchanged").await
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_missing_page_keeps_the_stored_text(pool: PgPool) -> anyhow::Result<()> {
    kept(&pool, GeniusTexts::Missing, "not_found").await
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn insufficient_metadata_keeps_the_stored_text(pool: PgPool) -> anyhow::Result<()> {
    kept(&pool, GeniusTexts::InsufficientMetadata, "no_metadata").await
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_blank_title_settles_without_a_lookup(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool, "42", " ", 0).await?;
    let job = sweep_job(&pool).await?;
    let handler = handler(&pool)?;
    let claim = claim_one(&pool, &job).await?;
    let before = version(&pool, "42").await?;

    handler.process_refetch(&job, claim).await?;

    assert_eq!(version(&pool, "42").await?, before);
    assert_eq!(
        ledger(&pool, "42").await?,
        ("done".to_owned(), Some("no_metadata".to_owned()), 1, true)
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_missing_track_settles_as_no_metadata(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO lyrics_repair (sc_track_id, kind, cache_created_at) VALUES ('42', 'refetch', $1::timestamp)",
    )
    .bind(CREATED)
    .execute(&pool)
    .await?;
    let job = sweep_job(&pool).await?;
    let handler = handler(&pool)?;
    let claim = claim_one(&pool, &job).await?;

    handler.process_refetch(&job, claim).await?;

    assert_eq!(
        ledger(&pool, "42").await?,
        ("done".to_owned(), Some("no_metadata".to_owned()), 1, true)
    );
    Ok(())
}

async fn stale_after(pool: &PgPool, change: &str) -> anyhow::Result<()> {
    seed(pool, "42", "Track", 0).await?;
    let job = sweep_job(pool).await?;
    let handler = handler(pool)?;
    let claim = claim_one(pool, &job).await?;
    sqlx::raw_sql(change).execute(pool).await?;
    let before = version(pool, "42").await.ok();

    handler.settle_refetch(&job, &claim, linked(NEW)).await?;

    assert_eq!(version(pool, "42").await.ok(), before);
    assert_eq!(
        ledger(pool, "42").await?,
        ("done".to_owned(), Some("stale".to_owned()), 1, true)
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_recreated_cache_row_is_stale(pool: PgPool) -> anyhow::Result<()> {
    stale_after(
        &pool,
        "UPDATE lyrics_cache SET created_at = created_at + interval '1 day'",
    )
    .await
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_cache_row_of_another_source_is_stale(pool: PgPool) -> anyhow::Result<()> {
    stale_after(
        &pool,
        "UPDATE lyrics_cache SET source = 'lrclib', plain_source = 'lrclib'",
    )
    .await
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_deleted_cache_row_is_stale(pool: PgPool) -> anyhow::Result<()> {
    stale_after(&pool, "DELETE FROM lyrics_cache").await
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_replace_over_a_changed_generation_is_stale(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool, "42", "Track", 0).await?;
    let job = sweep_job(&pool).await?;
    let claim = claim_one(&pool, &job).await?;
    sqlx::query("UPDATE lyrics_cache SET content_generation = 5")
        .execute(&pool)
        .await?;

    let outcome = sqlx::query_file_scalar!(
        "queries/lyrics/refetch_replace.sql",
        job.id,
        job.generation,
        job.lease_id,
        &claim.sc_track_id,
        1_i64,
        NEW
    )
    .fetch_one(&pool)
    .await?;

    let cache: (String, i64) =
        sqlx::query_as("SELECT plain_text, content_generation FROM lyrics_cache")
            .fetch_one(&pool)
            .await?;
    assert_eq!(outcome, "stale");
    assert_eq!(cache, (OLD.to_owned(), 5));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_failed_lookup_is_retried_in_an_hour(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool, "42", "Track", 0).await?;
    let job = sweep_job(&pool).await?;
    let handler = handler(&pool)?;
    let claim = claim_one(&pool, &job).await?;

    handler
        .settle_refetch(&job, &claim, GeniusTexts::Retry(failure()))
        .await?;

    let due: bool = sqlx::query_scalar(
        "SELECT next_run_at BETWEEN now() + interval '59 minutes' AND now() + interval '61 minutes'
         FROM lyrics_repair WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        ledger(&pool, "42").await?,
        ("pending".to_owned(), None, 1, true)
    );
    assert!(due);
    assert_eq!(due_refetches(&pool, &job).await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_sixth_failed_lookup_ends_as_failed(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool, "42", "Track", 0).await?;
    sqlx::query("UPDATE lyrics_repair SET attempts = 5")
        .execute(&pool)
        .await?;
    let job = sweep_job(&pool).await?;
    let handler = handler(&pool)?;
    let claim = claim_one(&pool, &job).await?;

    handler
        .settle_refetch(&job, &claim, GeniusTexts::Retry(failure()))
        .await?;

    assert_eq!(
        ledger(&pool, "42").await?,
        ("done".to_owned(), Some("failed".to_owned()), 6, true)
    );
    Ok(())
}

async fn untouched_by_a_fenced_out_settle(pool: PgPool, fence: &str) -> anyhow::Result<()> {
    seed(&pool, "42", "Track", 0).await?;
    let job = sweep_job(&pool).await?;
    let handler = handler(&pool)?;
    let claim = claim_one(&pool, &job).await?;
    sqlx::raw_sql(fence).execute(&pool).await?;
    let before = version(&pool, "42").await?;

    handler.settle_refetch(&job, &claim, linked(NEW)).await?;
    handler
        .settle_refetch(&job, &claim, GeniusTexts::Missing)
        .await?;

    let held: (String, bool) = sqlx::query_as(
        "SELECT status, claim_job_id IS NULL FROM lyrics_repair WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(version(&pool, "42").await?, before);
    assert_eq!(held, ("pending".to_owned(), false));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_expired_claim_writes_nothing(pool: PgPool) -> anyhow::Result<()> {
    untouched_by_a_fenced_out_settle(
        pool,
        "UPDATE lyrics_repair SET claim_expires_at = now() - interval '1 second'",
    )
    .await
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn another_lease_writes_nothing(pool: PgPool) -> anyhow::Result<()> {
    untouched_by_a_fenced_out_settle(
        pool,
        "UPDATE background_jobs SET lease_id = gen_random_uuid() WHERE kind = 'lyrics.lookup_sweep'",
    )
    .await
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn expired_claims_return_to_pending_or_are_abandoned_at_the_cap(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool, "41", "Track", 0).await?;
    seed(&pool, "42", "Track", 0).await?;
    sqlx::query("UPDATE lyrics_repair SET attempts = 5 WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;
    let job = sweep_job(&pool).await?;
    let handler = handler(&pool)?;
    let claimed =
        super::tiered::claim(&pool, &job, 10, 100, 900, Turn::led_by(Tier::StarLiked)).await?;
    assert_eq!(claimed.len(), 2);
    sqlx::query("UPDATE lyrics_repair SET claim_expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await?;

    handler.release_expired_refetches().await?;

    assert_eq!(
        ledger(&pool, "41").await?,
        ("pending".to_owned(), None, 1, true)
    );
    assert_eq!(
        ledger(&pool, "42").await?,
        ("done".to_owned(), Some("abandoned".to_owned()), 6, true)
    );
    Ok(())
}

async fn seed_many(
    pool: &PgPool,
    refetches: &[(&str, i16)],
    lookups: &[&str],
) -> anyhow::Result<()> {
    for (id, priority) in refetches {
        seed(pool, id, "Track", *priority).await?;
    }
    for id in lookups {
        sqlx::query(
            "INSERT INTO tracks (
                 sc_track_id, urn, title, title_normalized, uploader_username, duration_ms, index_priority
             ) VALUES ($1, 'soundcloud:tracks:' || $1, 'Track', 'track', 'artist', 180000, 5)",
        )
        .bind(id)
        .execute(pool)
        .await?;
    }
    sqlx::query("UPDATE lyrics_lookup_state SET next_run_at = now() - interval '1 hour'")
        .execute(pool)
        .await?;
    Ok(())
}

async fn split(
    pool: &PgPool,
    job: &LeasedJob,
    total: i64,
    share: i64,
) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let works =
        super::tiered::claim(pool, job, total, share, 900, Turn::led_by(Tier::StarLiked)).await?;
    let mut refetches = Vec::new();
    let mut lookups = Vec::new();
    for work in &works {
        match work {
            Work::Refetch(claim) => refetches.push(claim.sc_track_id.clone()),
            Work::Lookup(claim) => lookups.push(claim.sc_track_id.clone()),
        }
    }
    refetches.sort();
    lookups.sort();
    Ok((refetches, lookups))
}

const REFETCHES: [(&str, i16); 4] = [("201", 1), ("202", 9), ("203", 5), ("204", 7)];
const LOOKUPS: [&str; 4] = ["101", "102", "103", "104"];

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_zero_share_claims_no_refetch(pool: PgPool) -> anyhow::Result<()> {
    seed_many(&pool, &REFETCHES, &LOOKUPS).await?;
    let job = sweep_job(&pool).await?;

    let (refetches, lookups) = split(&pool, &job, 4, 0).await?;

    assert!(refetches.is_empty());
    assert_eq!(lookups, ["101", "102", "103", "104"]);
    let held: i64 =
        sqlx::query_scalar("SELECT count(*) FROM lyrics_repair WHERE claim_job_id IS NOT NULL")
            .fetch_one(&pool)
            .await?;
    assert_eq!(held, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn half_the_batch_goes_to_the_most_important_refetches(pool: PgPool) -> anyhow::Result<()> {
    seed_many(&pool, &REFETCHES, &LOOKUPS).await?;
    let job = sweep_job(&pool).await?;

    let (refetches, lookups) = split(&pool, &job, 4, 50).await?;

    assert_eq!(refetches, ["202", "204"]);
    assert_eq!(lookups.len(), 2);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_full_share_claims_only_refetches_until_the_ledger_is_drained(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_many(&pool, &REFETCHES, &LOOKUPS).await?;
    let job = sweep_job(&pool).await?;

    let (refetches, lookups) = split(&pool, &job, 3, 100).await?;
    assert_eq!(refetches.len(), 3);
    assert!(lookups.is_empty());

    let (refetches, lookups) = split(&pool, &job, 3, 100).await?;
    assert_eq!(refetches.len(), 1);
    assert_eq!(lookups.len(), 2);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_unused_refetch_share_goes_back_to_the_lookups(pool: PgPool) -> anyhow::Result<()> {
    seed_many(&pool, &[("201", 1)], &LOOKUPS).await?;
    let job = sweep_job(&pool).await?;

    let (refetches, lookups) = split(&pool, &job, 4, 50).await?;

    assert_eq!(refetches, ["201"]);
    assert_eq!(lookups.len(), 3);
    Ok(())
}
