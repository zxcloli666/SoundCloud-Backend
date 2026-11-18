use std::time::Duration;

use backend_contracts::{AdminMaintenancePayload, JobKind};
use sqlx::PgPool;
use uuid::Uuid;

use super::super::ARTIST_PLAYABLE_TRACKS;
use super::super::tests::handler;
use crate::queue::{ClaimOrder, JobRepository, NewJob};

const DRIFT: &str = "SELECT (
        SELECT count(*)
        FROM tracks AS track
        JOIN track_artists AS credit ON credit.track_id = track.id AND credit.role = 'primary'
        LEFT JOIN artist_playable_tracks AS playable
               ON playable.artist_id = credit.artist_id AND playable.track_id = track.id
        WHERE track.sharing = 'public'
          AND track.superseded_by IS NULL
          AND track.storage_state = 'ok'
          AND (track.needs_duration_resolve = false OR track.index_state = 'indexed')
          AND (playable.track_id IS NULL
               OR (track.sc_track_id, track.needs_duration_resolve = false, track.index_state = 'indexed')
                  IS DISTINCT FROM
                  (playable.sc_track_id, playable.ready_home, playable.ready_wave))
    ) + (
        SELECT count(*)
        FROM artist_playable_tracks AS playable
        WHERE NOT EXISTS (
            SELECT 1
            FROM tracks AS track
            JOIN track_artists AS credit ON credit.track_id = track.id AND credit.role = 'primary'
            WHERE track.id = playable.track_id
              AND credit.artist_id = playable.artist_id
              AND track.sharing = 'public'
              AND track.superseded_by IS NULL
              AND track.storage_state = 'ok'
              AND (track.needs_duration_resolve = false OR track.index_state = 'indexed')
        )
    )";

#[derive(Debug, sqlx::FromRow)]
struct Totals {
    status: String,
    scanned: i64,
    changed: i64,
    merged: i64,
    skipped: i64,
}

type Row = (String, String, bool, bool);

fn row(track: &str, artist: &str, home: bool, wave: bool) -> Row {
    (track.to_owned(), artist.to_owned(), home, wave)
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_triggers_keep_the_table_equal_to_the_playable_primary_rows(
    pool: PgPool,
) -> anyhow::Result<()> {
    let first = artist(&pool, "a1").await?;
    let second = artist(&pool, "a2").await?;
    let one = track(&pool, "s1", "public", "pending", false, "pending").await?;
    let two = track(&pool, "s2", "public", "pending", false, "pending").await?;
    credit(&pool, one, first, "primary").await?;
    credit(&pool, two, first, "primary").await?;
    credit(&pool, two, second, "featured").await?;
    assert_eq!(playable(&pool).await?, []);

    execute(&pool, "UPDATE tracks SET storage_state = 'ok'").await?;
    assert_eq!(
        playable(&pool).await?,
        [row("s1", "a1", true, false), row("s2", "a1", true, false)]
    );
    assert_eq!(drift(&pool).await?, 0);

    execute_for(
        &pool,
        two,
        "UPDATE tracks SET needs_duration_resolve = true WHERE id = $1",
    )
    .await?;
    assert_eq!(playable(&pool).await?, [row("s1", "a1", true, false)]);

    execute_for(
        &pool,
        two,
        "UPDATE tracks SET index_state = 'indexed' WHERE id = $1",
    )
    .await?;
    assert_eq!(
        playable(&pool).await?,
        [row("s1", "a1", true, false), row("s2", "a1", false, true)]
    );

    execute_for(
        &pool,
        two,
        "UPDATE tracks SET needs_duration_resolve = false WHERE id = $1",
    )
    .await?;
    assert_eq!(
        playable(&pool).await?,
        [row("s1", "a1", true, false), row("s2", "a1", true, true)]
    );
    assert_eq!(drift(&pool).await?, 0);

    execute_for(
        &pool,
        one,
        "UPDATE tracks SET sharing = 'private' WHERE id = $1",
    )
    .await?;
    assert_eq!(playable(&pool).await?, [row("s2", "a1", true, true)]);
    execute_for(
        &pool,
        one,
        "UPDATE tracks SET sharing = 'public' WHERE id = $1",
    )
    .await?;
    assert_eq!(playable(&pool).await?.len(), 2);

    sqlx::query("UPDATE tracks SET superseded_by = $2 WHERE id = $1")
        .bind(one)
        .bind(two)
        .execute(&pool)
        .await?;
    assert_eq!(playable(&pool).await?, [row("s2", "a1", true, true)]);
    execute_for(
        &pool,
        one,
        "UPDATE tracks SET superseded_by = NULL WHERE id = $1",
    )
    .await?;
    assert_eq!(playable(&pool).await?.len(), 2);

    execute_for(
        &pool,
        one,
        "UPDATE tracks SET storage_state = 'missing' WHERE id = $1",
    )
    .await?;
    assert_eq!(playable(&pool).await?, [row("s2", "a1", true, true)]);
    execute_for(
        &pool,
        one,
        "UPDATE tracks SET storage_state = 'ok' WHERE id = $1",
    )
    .await?;
    assert_eq!(drift(&pool).await?, 0);
    assert_eq!(playable(&pool).await?.len(), 2);

    let before = row_version(&pool, two).await?;
    execute_for(
        &pool,
        two,
        "UPDATE tracks SET title = 'Other title' WHERE id = $1",
    )
    .await?;
    assert_eq!(row_version(&pool, two).await?, before);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_triggers_follow_track_artists_inserts_deletes_and_role_changes(
    pool: PgPool,
) -> anyhow::Result<()> {
    let first = artist(&pool, "a1").await?;
    let second = artist(&pool, "a2").await?;
    let one = track(&pool, "s1", "public", "ok", false, "pending").await?;
    let two = track(&pool, "s2", "public", "ok", true, "indexed").await?;
    assert_eq!(playable(&pool).await?, []);

    credit(&pool, one, first, "primary").await?;
    credit(&pool, one, second, "featured").await?;
    credit(&pool, two, second, "primary").await?;
    assert_eq!(
        playable(&pool).await?,
        [row("s1", "a1", true, false), row("s2", "a2", false, true)]
    );

    execute_credit(
        &pool,
        one,
        second,
        "UPDATE track_artists SET role = 'primary' WHERE track_id = $1 AND artist_id = $2",
    )
    .await?;
    assert_eq!(
        playable(&pool).await?,
        [
            row("s1", "a1", true, false),
            row("s1", "a2", true, false),
            row("s2", "a2", false, true)
        ]
    );

    execute_credit(
        &pool,
        one,
        first,
        "UPDATE track_artists SET role = 'featured' WHERE track_id = $1 AND artist_id = $2",
    )
    .await?;
    assert_eq!(
        playable(&pool).await?,
        [row("s1", "a2", true, false), row("s2", "a2", false, true)]
    );

    execute_credit(
        &pool,
        two,
        second,
        "DELETE FROM track_artists WHERE track_id = $1 AND artist_id = $2",
    )
    .await?;
    assert_eq!(playable(&pool).await?, [row("s1", "a2", true, false)]);
    assert_eq!(drift(&pool).await?, 0);

    execute_for(&pool, one, "DELETE FROM tracks WHERE id = $1").await?;
    assert_eq!(playable(&pool).await?, []);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_backfill_fills_rows_that_existed_before_the_triggers(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let expected = playable(&pool).await?;
    assert_eq!(expected.len(), 4);
    execute(&pool, "DELETE FROM artist_playable_tracks").await?;
    assert!(drift(&pool).await? > 0);

    let totals = run(&pool).await?;

    assert_eq!(totals.status, "completed");
    assert_eq!(totals.changed, 4);
    assert_eq!(totals.merged, 0);
    assert_eq!(totals.skipped, 0);
    assert!(totals.scanned > 0);
    assert_eq!(drift(&pool).await?, 0);
    assert_eq!(playable(&pool).await?, expected);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_check_reports_nothing_after_a_backfill(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool).await?;
    execute(&pool, "DELETE FROM artist_playable_tracks").await?;

    let totals = run(&pool).await?;

    assert_eq!(totals.status, "completed");
    assert_eq!(totals.skipped, 0);
    assert_eq!(totals.merged, 0);
    assert_eq!(drift(&pool).await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_repair_fixes_wrong_extra_and_missing_rows(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool).await?;
    let expected = playable(&pool).await?;
    execute(
        &pool,
        "UPDATE artist_playable_tracks SET ready_wave = NOT ready_wave
         WHERE sc_track_id = 's1'",
    )
    .await?;
    execute(
        &pool,
        "INSERT INTO artist_playable_tracks (artist_id, track_id, sc_track_id, ready_home, ready_wave)
         SELECT artist.id, track.id, track.sc_track_id, true, false
         FROM tracks AS track
         CROSS JOIN artists AS artist
         WHERE (track.sc_track_id, artist.name) IN (('s3', 'a1'), ('s2', 'a1'))",
    )
    .await?;
    assert_eq!(drift(&pool).await?, 3);

    let totals = run(&pool).await?;

    assert_eq!(totals.status, "completed");
    assert_eq!(totals.skipped, 0);
    assert_eq!(totals.merged, 2);
    assert_eq!(drift(&pool).await?, 0);
    assert_eq!(playable(&pool).await?, expected);
    Ok(())
}

async fn run(pool: &PgPool) -> anyhow::Result<Totals> {
    let run_id = Uuid::now_v7();
    let queue = JobRepository::new(pool.clone(), "artist-playable-tracks-admin-test".to_owned());
    queue.enqueue_if_absent(&job(run_id)).await?;
    let job = queue
        .claim(
            &[JobKind::AdminArtistPlayableTracks],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("artist playable tracks job was not claimed"))?;
    let handler = handler(pool);
    for _ in 0..500 {
        handler
            .backfill_artist_playable_tracks(&job, AdminMaintenancePayload { run_id })
            .await?;
        let totals: Totals = sqlx::query_as(
            "SELECT status, scanned, changed, merged, skipped
             FROM admin_maintenance_runs
             WHERE kind = $1",
        )
        .bind(ARTIST_PLAYABLE_TRACKS)
        .fetch_one(pool)
        .await?;
        if totals.status == "completed" {
            return Ok(totals);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    anyhow::bail!("the artist playable tracks run did not complete")
}

fn job(run_id: Uuid) -> NewJob {
    NewJob {
        id: Uuid::now_v7(),
        kind: JobKind::AdminArtistPlayableTracks,
        dedup_key: Some(ARTIST_PLAYABLE_TRACKS.to_owned()),
        payload: serde_json::json!({
            "version": "1",
            "payload": { "run_id": run_id }
        }),
        priority: 20,
        max_attempts: 8,
        available_at: chrono::Utc::now() - chrono::Duration::minutes(1),
    }
}

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    let first = artist(pool, "a1").await?;
    let second = artist(pool, "a2").await?;
    let one = track(pool, "s1", "public", "ok", false, "pending").await?;
    let two = track(pool, "s2", "public", "ok", true, "indexed").await?;
    let three = track(pool, "s3", "private", "ok", false, "pending").await?;
    let four = track(pool, "s4", "public", "ok", false, "indexed").await?;
    credit(pool, one, first, "primary").await?;
    credit(pool, one, second, "featured").await?;
    credit(pool, two, second, "primary").await?;
    credit(pool, three, first, "primary").await?;
    credit(pool, four, first, "primary").await?;
    credit(pool, four, second, "primary").await?;
    Ok(())
}

async fn artist(pool: &PgPool, name: &str) -> anyhow::Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO artists (name, normalized_name, source)
         VALUES ($1, $1, 'sc')
         RETURNING id",
    )
    .bind(name)
    .fetch_one(pool)
    .await?)
}

async fn track(
    pool: &PgPool,
    sc_track_id: &str,
    sharing: &str,
    storage_state: &str,
    needs_duration_resolve: bool,
    index_state: &str,
) -> anyhow::Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms,
             sharing, storage_state, needs_duration_resolve, index_state
         )
         VALUES ($1, 'soundcloud:tracks:' || $1, 'Track', 'track', 180000, $2, $3, $4, $5)
         RETURNING id",
    )
    .bind(sc_track_id)
    .bind(sharing)
    .bind(storage_state)
    .bind(needs_duration_resolve)
    .bind(index_state)
    .fetch_one(pool)
    .await?)
}

async fn credit(pool: &PgPool, track: Uuid, artist: Uuid, role: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO track_artists (track_id, artist_id, role, source)
         VALUES ($1, $2, $3, 'sc')",
    )
    .bind(track)
    .bind(artist)
    .bind(role)
    .execute(pool)
    .await?;
    Ok(())
}

async fn playable(pool: &PgPool) -> anyhow::Result<Vec<Row>> {
    Ok(sqlx::query_as(
        "SELECT playable.sc_track_id, artist.name, playable.ready_home, playable.ready_wave
         FROM artist_playable_tracks AS playable
         JOIN artists AS artist ON artist.id = playable.artist_id
         ORDER BY playable.sc_track_id, artist.name",
    )
    .fetch_all(pool)
    .await?)
}

async fn row_version(pool: &PgPool, track: Uuid) -> anyhow::Result<String> {
    Ok(sqlx::query_scalar(
        "SELECT string_agg(xmin::text, ',' ORDER BY artist_id)
         FROM artist_playable_tracks
         WHERE track_id = $1",
    )
    .bind(track)
    .fetch_one(pool)
    .await?)
}

async fn execute(pool: &PgPool, sql: &str) -> anyhow::Result<()> {
    sqlx::query(sql).execute(pool).await?;
    Ok(())
}

async fn execute_for(pool: &PgPool, id: Uuid, sql: &str) -> anyhow::Result<()> {
    sqlx::query(sql).bind(id).execute(pool).await?;
    Ok(())
}

async fn execute_credit(
    pool: &PgPool,
    track: Uuid,
    artist: Uuid,
    sql: &str,
) -> anyhow::Result<()> {
    sqlx::query(sql)
        .bind(track)
        .bind(artist)
        .execute(pool)
        .await?;
    Ok(())
}

async fn drift(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(DRIFT).fetch_one(pool).await?)
}
