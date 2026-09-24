use std::time::Duration;

use backend_contracts::{AdminMaintenancePayload, JobKind};
use catalog_sources::{ExternalFetcher, MbClient};
use sqlx::PgPool;
use uuid::Uuid;

use super::{AdminMaintenanceHandler, CATALOG_RENORMALIZE};
use crate::queue::{ClaimOrder, JobRepository, LeasedJob, NewJob};

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE artists (
             id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
             name text NOT NULL,
             normalized_name text NOT NULL,
             mb_artist_id text,
             genius_artist_id text,
             merged_into uuid
         );
         CREATE UNIQUE INDEX artists_normalized_idx
             ON artists (normalized_name) WHERE merged_into IS NULL;
         CREATE TABLE tracks (
             id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
             title text NOT NULL,
             title_normalized text NOT NULL,
             metadata_artist text,
             primary_artist_id uuid,
             cover_of_artist_id uuid
         );
         CREATE TABLE albums (
             id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
             title text NOT NULL,
             normalized_title text NOT NULL,
             primary_artist_id uuid
         );
         CREATE TABLE playlists (
             urn text PRIMARY KEY,
             title text NOT NULL,
             title_normalized text NOT NULL
         );
         CREATE TABLE track_artists (
             track_id uuid NOT NULL,
             artist_id uuid NOT NULL,
             role text NOT NULL,
             PRIMARY KEY (track_id, artist_id, role)
         );
         CREATE TABLE album_artists (
             album_id uuid NOT NULL,
             artist_id uuid NOT NULL,
             role text NOT NULL,
             PRIMARY KEY (album_id, artist_id, role)
         );",
    )
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0057_background_jobs.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0083_admin_maintenance_runs.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

fn handler(pool: &PgPool) -> AdminMaintenanceHandler {
    let http = sc_fingerprint::builder(None)
        .timeout(Duration::from_secs(1))
        .build()
        .expect("http client");
    let fetcher = ExternalFetcher::new(http, String::new(), None);
    AdminMaintenanceHandler::new(
        pool.clone(),
        MbClient::new(fetcher, 1100),
        crate::config::AdminMaintenanceConfig { scan_batch: 2_000 },
    )
}

async fn claim(pool: &PgPool, run_id: Uuid) -> anyhow::Result<LeasedJob> {
    let queue = JobRepository::new(pool.clone(), "admin-test".to_owned());
    queue
        .enqueue_if_absent(&NewJob {
            id: Uuid::now_v7(),
            kind: JobKind::AdminCatalogRenormalize,
            dedup_key: Some(CATALOG_RENORMALIZE.to_owned()),
            payload: serde_json::json!({
                "version": "1",
                "payload": { "run_id": run_id }
            }),
            priority: 20,
            max_attempts: 8,
            available_at: chrono::Utc::now(),
        })
        .await?;
    let mut jobs = queue
        .claim(
            &[JobKind::AdminCatalogRenormalize],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?;
    jobs.pop()
        .ok_or_else(|| anyhow::anyhow!("maintenance job was not claimed"))
}

#[sqlx::test(migrations = false)]
async fn renormalize_resumes_from_its_persisted_cursor(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    for index in 0..3 {
        sqlx::query("INSERT INTO artists (name, normalized_name) VALUES ($1, $2)")
            .bind(format!("ᴀʀᴛɪsᴛ {index}"))
            .bind(format!("stale {index}"))
            .execute(&pool)
            .await?;
    }
    let run_id = Uuid::now_v7();
    let job = claim(&pool, run_id).await?;
    let handler = handler(&pool);

    handler
        .renormalize_catalog(&job, AdminMaintenancePayload { run_id })
        .await?;

    let state: (String, String, i64, Option<Uuid>) = sqlx::query_as(
        "SELECT status, phase, changed, cursor_uuid
         FROM admin_maintenance_runs WHERE kind = 'catalog_renormalize'",
    )
    .fetch_one(&pool)
    .await?;
    let continuation: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM background_jobs WHERE kind = 'admin.renormalize_catalog'",
    )
    .fetch_one(&pool)
    .await?;

    assert_eq!(state.0, "running");
    assert_eq!(state.1, "artists");
    assert_eq!(state.2, 3);
    assert!(state.3.is_some());
    assert_eq!(continuation, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn stale_run_identity_cannot_touch_a_newer_run(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query("INSERT INTO artists (name, normalized_name) VALUES ('ᴍᴏɴᴀʀᴄʜ', 'stale')")
        .execute(&pool)
        .await?;
    let current = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO admin_maintenance_runs (kind, run_id, status, phase)
         VALUES ('catalog_renormalize', $1, 'running', 'artists')",
    )
    .bind(current)
    .execute(&pool)
    .await?;
    let stale = Uuid::now_v7();
    let job = claim(&pool, stale).await?;

    handler(&pool)
        .renormalize_catalog(&job, AdminMaintenancePayload { run_id: stale })
        .await?;

    let untouched: (Uuid, i64) = sqlx::query_as(
        "SELECT run_id, scanned FROM admin_maintenance_runs WHERE kind = 'catalog_renormalize'",
    )
    .fetch_one(&pool)
    .await?;
    let normalized: String = sqlx::query_scalar("SELECT normalized_name FROM artists LIMIT 1")
        .fetch_one(&pool)
        .await?;

    assert_eq!(untouched, (current, 0));
    assert_eq!(normalized, "stale");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn empty_phase_advances_and_last_phase_completes_the_run(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let run_id = Uuid::now_v7();
    let handler = handler(&pool);

    for expected in [
        "repoint",
        "roles",
        "track_titles",
        "album_titles",
        "playlist_titles",
        "track_meta",
    ] {
        let job = claim(&pool, run_id).await?;
        handler
            .renormalize_catalog(&job, AdminMaintenancePayload { run_id })
            .await?;
        let phase: String = sqlx::query_scalar(
            "SELECT phase FROM admin_maintenance_runs WHERE kind = 'catalog_renormalize'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(phase, expected);
        sqlx::query("DELETE FROM background_jobs WHERE kind = 'admin.renormalize_catalog'")
            .execute(&pool)
            .await?;
    }

    let job = claim(&pool, run_id).await?;
    handler
        .renormalize_catalog(&job, AdminMaintenancePayload { run_id })
        .await?;

    let state: (String, String, bool) = sqlx::query_as(
        "SELECT status, phase, completed_at IS NOT NULL
         FROM admin_maintenance_runs WHERE kind = 'catalog_renormalize'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(state, ("completed".to_owned(), "done".to_owned(), true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn mismatched_dedup_identity_is_rejected(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let run_id = Uuid::now_v7();
    let mut job = claim(&pool, run_id).await?;
    job.dedup_key = Some("musicbrainz_names".to_owned());

    let error = handler(&pool)
        .renormalize_catalog(&job, AdminMaintenancePayload { run_id })
        .await
        .expect_err("identity mismatch must be rejected");

    assert!(!error.is_retryable());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn merged_artist_references_are_repointed_to_the_root(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let holder: Uuid = sqlx::query_scalar(
        "INSERT INTO artists (name, normalized_name) VALUES ('holder', 'holder') RETURNING id",
    )
    .fetch_one(&pool)
    .await?;
    let alias: Uuid = sqlx::query_scalar(
        "INSERT INTO artists (name, normalized_name, merged_into)
         VALUES ('alias', 'alias', $1) RETURNING id",
    )
    .bind(holder)
    .fetch_one(&pool)
    .await?;
    let track: Uuid = sqlx::query_scalar(
        "INSERT INTO tracks (title, title_normalized, primary_artist_id)
         VALUES ('song', 'song', $1) RETURNING id",
    )
    .bind(alias)
    .fetch_one(&pool)
    .await?;
    sqlx::query("INSERT INTO track_artists (track_id, artist_id, role) VALUES ($1, $2, 'primary')")
        .bind(track)
        .bind(alias)
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO admin_maintenance_runs (kind, run_id, status, phase)
         VALUES ('catalog_renormalize', $1, 'running', 'repoint')",
    )
    .bind(Uuid::nil())
    .execute(&pool)
    .await?;
    let run_id = Uuid::nil();
    let job = claim(&pool, run_id).await?;

    handler(&pool)
        .renormalize_catalog(&job, AdminMaintenancePayload { run_id })
        .await?;

    let primary: Option<Uuid> =
        sqlx::query_scalar("SELECT primary_artist_id FROM tracks WHERE id = $1")
            .bind(track)
            .fetch_one(&pool)
            .await?;
    let credited: Option<Uuid> =
        sqlx::query_scalar("SELECT artist_id FROM track_artists WHERE track_id = $1")
            .bind(track)
            .fetch_optional(&pool)
            .await?;

    assert_eq!(primary, Some(holder));
    assert_eq!(credited, Some(holder));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_slice_leaves_a_live_continuation_behind_the_job_it_is_still_running(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    for index in 0..3 {
        sqlx::query("INSERT INTO artists (name, normalized_name) VALUES ($1, $2)")
            .bind(format!("ᴀʀᴛɪsᴛ {index}"))
            .bind(format!("stale {index}"))
            .execute(&pool)
            .await?;
    }
    let run_id = Uuid::now_v7();
    let job = claim(&pool, run_id).await?;
    let handler = handler(&pool);

    handler
        .renormalize_catalog(&job, AdminMaintenancePayload { run_id })
        .await?;

    let continuation: (i64, i64) = sqlx::query_as(
        "SELECT count(*), coalesce(max(generation), 0)
         FROM background_jobs
         WHERE kind = $1 AND dedup_key = $2",
    )
    .bind(JobKind::AdminCatalogRenormalize.as_str())
    .bind(CATALOG_RENORMALIZE)
    .fetch_one(&pool)
    .await?;

    assert_eq!(continuation.0, 1);
    assert!(continuation.1 > job.generation);
    Ok(())
}
