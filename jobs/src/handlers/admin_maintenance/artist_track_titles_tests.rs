use std::time::Duration;

use backend_contracts::{AdminMaintenancePayload, JobKind};
use sqlx::PgPool;
use uuid::Uuid;

use super::super::ARTIST_TRACK_TITLES;
use super::super::tests::handler;
use crate::queue::{ClaimOrder, JobRepository, NewJob};

const DRIFT: &str = "WITH expected AS (
        SELECT track.primary_artist_id AS artist_id, track.title_normalized COLLATE \"C\" AS title_normalized, track.id AS track_id
        FROM tracks AS track
        WHERE track.superseded_by IS NULL
          AND track.title_normalized IS NOT NULL
          AND track.primary_artist_id IS NOT NULL
        UNION
        SELECT credit.artist_id, track.title_normalized COLLATE \"C\", track.id
        FROM tracks AS track
        JOIN track_artists AS credit ON credit.track_id = track.id AND credit.role = 'primary'
        WHERE track.superseded_by IS NULL
          AND track.title_normalized IS NOT NULL
    )
    SELECT count(*)
    FROM expected
    FULL JOIN artist_track_titles AS stored
           ON stored.artist_id = expected.artist_id
          AND stored.title_normalized = expected.title_normalized
          AND stored.track_id = expected.track_id
    WHERE expected.track_id IS NULL OR stored.track_id IS NULL";

#[derive(Debug, sqlx::FromRow)]
struct Totals {
    status: String,
    scanned: i64,
    changed: i64,
    merged: i64,
    skipped: i64,
}

type Row = (String, String, String);

fn row(track: &str, artist: &str, title: &str) -> Row {
    (track.to_owned(), artist.to_owned(), title.to_owned())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_triggers_follow_the_title_the_primary_artist_and_superseded_of_a_track(
    pool: PgPool,
) -> anyhow::Result<()> {
    let first = artist(&pool, "a1").await?;
    let second = artist(&pool, "a2").await?;
    let one = track(&pool, "s1", Some(first), Some("first song")).await?;
    let two = track(&pool, "s2", None, Some("second song")).await?;
    let three = track(&pool, "s3", Some(second), None).await?;
    assert_eq!(titles(&pool).await?, [row("s1", "a1", "first song")]);

    execute_for(
        &pool,
        one,
        "UPDATE tracks SET title_normalized = 'first song live' WHERE id = $1",
    )
    .await?;
    assert_eq!(titles(&pool).await?, [row("s1", "a1", "first song live")]);

    execute_pair(
        &pool,
        one,
        second,
        "UPDATE tracks SET primary_artist_id = $2 WHERE id = $1",
    )
    .await?;
    assert_eq!(titles(&pool).await?, [row("s1", "a2", "first song live")]);

    execute_pair(
        &pool,
        two,
        first,
        "UPDATE tracks SET primary_artist_id = $2 WHERE id = $1",
    )
    .await?;
    execute_for(
        &pool,
        three,
        "UPDATE tracks SET title_normalized = 'third song' WHERE id = $1",
    )
    .await?;
    assert_eq!(
        titles(&pool).await?,
        [
            row("s1", "a2", "first song live"),
            row("s2", "a1", "second song"),
            row("s3", "a2", "third song")
        ]
    );
    assert_eq!(drift(&pool).await?, 0);

    execute_pair(
        &pool,
        one,
        two,
        "UPDATE tracks SET superseded_by = $2 WHERE id = $1",
    )
    .await?;
    assert_eq!(
        titles(&pool).await?,
        [
            row("s2", "a1", "second song"),
            row("s3", "a2", "third song")
        ]
    );
    execute_for(
        &pool,
        one,
        "UPDATE tracks SET superseded_by = NULL WHERE id = $1",
    )
    .await?;
    assert_eq!(titles(&pool).await?.len(), 3);

    execute_for(
        &pool,
        three,
        "UPDATE tracks SET title_normalized = NULL WHERE id = $1",
    )
    .await?;
    assert_eq!(titles(&pool).await?.len(), 2);

    let before = row_version(&pool, two).await?;
    execute_for(
        &pool,
        two,
        "UPDATE tracks SET title = 'Other title', sharing = 'private' WHERE id = $1",
    )
    .await?;
    assert_eq!(row_version(&pool, two).await?, before);

    execute_for(&pool, two, "DELETE FROM tracks WHERE id = $1").await?;
    assert_eq!(titles(&pool).await?, [row("s1", "a2", "first song live")]);
    assert_eq!(drift(&pool).await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_triggers_follow_primary_credits_and_keep_one_row_per_artist(
    pool: PgPool,
) -> anyhow::Result<()> {
    let first = artist(&pool, "a1").await?;
    let second = artist(&pool, "a2").await?;
    let third = artist(&pool, "a3").await?;
    let one = track(&pool, "s1", Some(first), Some("song")).await?;

    credit(&pool, one, first, "primary").await?;
    credit(&pool, one, second, "featured").await?;
    assert_eq!(titles(&pool).await?, [row("s1", "a1", "song")]);

    credit(&pool, one, third, "primary").await?;
    assert_eq!(
        titles(&pool).await?,
        [row("s1", "a1", "song"), row("s1", "a3", "song")]
    );

    execute_credit(
        &pool,
        one,
        second,
        "UPDATE track_artists SET role = 'primary' WHERE track_id = $1 AND artist_id = $2",
    )
    .await?;
    assert_eq!(titles(&pool).await?.len(), 3);

    execute_credit(
        &pool,
        one,
        third,
        "UPDATE track_artists SET role = 'featured' WHERE track_id = $1 AND artist_id = $2",
    )
    .await?;
    assert_eq!(
        titles(&pool).await?,
        [row("s1", "a1", "song"), row("s1", "a2", "song")]
    );

    execute_credit(
        &pool,
        one,
        first,
        "DELETE FROM track_artists WHERE track_id = $1 AND artist_id = $2",
    )
    .await?;
    assert_eq!(
        titles(&pool).await?,
        [row("s1", "a1", "song"), row("s1", "a2", "song")]
    );

    execute_credit(
        &pool,
        one,
        second,
        "DELETE FROM track_artists WHERE track_id = $1 AND artist_id = $2",
    )
    .await?;
    assert_eq!(titles(&pool).await?, [row("s1", "a1", "song")]);
    assert_eq!(drift(&pool).await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_backfill_fills_rows_that_existed_before_the_triggers(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let expected = titles(&pool).await?;
    assert_eq!(expected.len(), 4);
    execute(&pool, "DELETE FROM artist_track_titles").await?;
    assert!(drift(&pool).await? > 0);

    let totals = run(&pool).await?;

    assert_eq!(totals.status, "completed");
    assert_eq!(totals.changed, 4);
    assert_eq!(totals.merged, 0);
    assert_eq!(totals.skipped, 0);
    assert!(totals.scanned > 0);
    assert_eq!(drift(&pool).await?, 0);
    assert_eq!(titles(&pool).await?, expected);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_repair_fixes_wrong_extra_and_missing_rows(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool).await?;
    let expected = titles(&pool).await?;
    execute(
        &pool,
        "UPDATE artist_track_titles SET title_normalized = 'stale'
         WHERE track_id = (SELECT id FROM tracks WHERE sc_track_id = 's1')",
    )
    .await?;
    execute(
        &pool,
        "INSERT INTO artist_track_titles (artist_id, title_normalized, track_id)
         SELECT artist.id, 'old song', track.id
         FROM tracks AS track
         CROSS JOIN artists AS artist
         WHERE (track.sc_track_id, artist.name) IN (('s3', 'a1'), ('s2', 'a1'))",
    )
    .await?;
    execute(
        &pool,
        "DELETE FROM artist_track_titles
         WHERE track_id = (SELECT id FROM tracks WHERE sc_track_id = 's4')
           AND artist_id = (SELECT id FROM artists WHERE name = 'a2')",
    )
    .await?;
    assert_eq!(drift(&pool).await?, 5);

    let totals = run(&pool).await?;

    assert_eq!(totals.status, "completed");
    assert_eq!(totals.skipped, 0);
    assert_eq!(totals.merged, 3);
    assert_eq!(drift(&pool).await?, 0);
    assert_eq!(titles(&pool).await?, expected);
    Ok(())
}

async fn run(pool: &PgPool) -> anyhow::Result<Totals> {
    let run_id = Uuid::now_v7();
    let queue = JobRepository::new(pool.clone(), "artist-track-titles-admin-test".to_owned());
    queue.enqueue_if_absent(&job(run_id)).await?;
    let job = queue
        .claim(
            &[JobKind::AdminArtistTrackTitles],
            ClaimOrder::Priority,
            1,
            Duration::from_secs(600),
        )
        .await?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("artist track titles job was not claimed"))?;
    let handler = handler(pool);
    for _ in 0..500 {
        handler
            .backfill_artist_track_titles(&job, AdminMaintenancePayload { run_id })
            .await?;
        let totals: Totals = sqlx::query_as(
            "SELECT status, scanned, changed, merged, skipped
             FROM admin_maintenance_runs
             WHERE kind = $1",
        )
        .bind(ARTIST_TRACK_TITLES)
        .fetch_one(pool)
        .await?;
        if totals.status == "completed" {
            return Ok(totals);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    anyhow::bail!("the artist track titles run did not complete")
}

fn job(run_id: Uuid) -> NewJob {
    NewJob {
        id: Uuid::now_v7(),
        kind: JobKind::AdminArtistTrackTitles,
        dedup_key: Some(ARTIST_TRACK_TITLES.to_owned()),
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
    let one = track(pool, "s1", Some(first), Some("one")).await?;
    let two = track(pool, "s2", None, Some("two")).await?;
    let three = track(pool, "s3", Some(first), None).await?;
    let four = track(pool, "s4", Some(first), Some("four")).await?;
    credit(pool, one, first, "primary").await?;
    credit(pool, one, second, "featured").await?;
    credit(pool, two, second, "primary").await?;
    credit(pool, three, first, "primary").await?;
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
    primary_artist_id: Option<Uuid>,
    title_normalized: Option<&str>,
) -> anyhow::Result<Uuid> {
    Ok(sqlx::query_scalar(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, primary_artist_id, duration_ms, sharing
         )
         VALUES ($1, 'soundcloud:tracks:' || $1, 'Track', $3, $2, 180000, 'public')
         RETURNING id",
    )
    .bind(sc_track_id)
    .bind(primary_artist_id)
    .bind(title_normalized)
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

async fn titles(pool: &PgPool) -> anyhow::Result<Vec<Row>> {
    Ok(sqlx::query_as(
        "SELECT track.sc_track_id, artist.name, title.title_normalized::text
         FROM artist_track_titles AS title
         JOIN tracks AS track ON track.id = title.track_id
         JOIN artists AS artist ON artist.id = title.artist_id
         ORDER BY track.sc_track_id, artist.name",
    )
    .fetch_all(pool)
    .await?)
}

async fn row_version(pool: &PgPool, track: Uuid) -> anyhow::Result<String> {
    Ok(sqlx::query_scalar(
        "SELECT string_agg(xmin::text, ',' ORDER BY artist_id)
         FROM artist_track_titles
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

async fn execute_pair(pool: &PgPool, id: Uuid, other: Uuid, sql: &str) -> anyhow::Result<()> {
    sqlx::query(sql).bind(id).bind(other).execute(pool).await?;
    Ok(())
}

async fn execute_credit(pool: &PgPool, track: Uuid, artist: Uuid, sql: &str) -> anyhow::Result<()> {
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
