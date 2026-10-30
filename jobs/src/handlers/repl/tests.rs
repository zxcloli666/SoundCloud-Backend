use backend_contracts::ReplReplayPayload;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use super::ReplHandler;

async fn with_mode(pool: &PgPool, mode: &str) -> anyhow::Result<PgPool> {
    sqlx::query(
        "DO $$ BEGIN EXECUTE format('ALTER DATABASE %I SET scd.mode = %L', current_database(), $m$MODE$m$); END $$"
            .replace("MODE", mode)
            .as_str(),
    )
    .execute(pool)
    .await?;
    let options = (*pool.connect_options()).clone();
    Ok(PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await?)
}

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO cmd_done (node, id, status, at)
         SELECT 'main', gen_random_uuid(), 'ok', now() - age
         FROM unnest(ARRAY[interval '8 days', interval '30 days', interval '7 days 1 minute',
                           interval '6 days 23 hours', interval '1 hour']) AS age;
         INSERT INTO cmd_dead (node, id, kind, args, sqlstate, message, at)
         SELECT 'main', gen_random_uuid(), 'track.edit', '{}', '23514', 'seed', now() - age
         FROM unnest(ARRAY[interval '8 days', interval '6 days 23 hours']) AS age;
         INSERT INTO repl_purges (tbl, pk, purged_v, at)
         SELECT 'user_likes_tracks', jsonb_build_object('user_id', 'u', 'sc_track_id', gen_random_uuid()), 1, now() - age
         FROM unnest(ARRAY[interval '8 days', interval '2 days']) AS age;
         INSERT INTO repl_tombstones (tbl, pk, at)
         SELECT 'disliked_tracks', jsonb_build_object('id', gen_random_uuid()), now() - age
         FROM unnest(ARRAY[interval '8 days', interval '9 days', interval '2 days']) AS age",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn counts(pool: &PgPool) -> anyhow::Result<(i64, i64, i64, i64)> {
    Ok(sqlx::query_as(
        "SELECT (SELECT count(*) FROM cmd_done), (SELECT count(*) FROM cmd_dead),
                (SELECT count(*) FROM repl_purges), (SELECT count(*) FROM repl_tombstones)",
    )
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrations = "../api/migrations")]
async fn retention_deletes_the_rows_past_seven_days_and_keeps_the_rest(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    assert_eq!(counts(&pool).await?, (5, 2, 2, 3));

    ReplHandler::new(pool.clone()).retain().await?;

    assert_eq!(counts(&pool).await?, (2, 1, 1, 1));
    let youngest_kept: i64 =
        sqlx::query_scalar("SELECT count(*) FROM cmd_done WHERE at > now() - interval '7 days'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(youngest_kept, 2);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_split_core_without_its_peers_keeps_tombstones_receipts_and_acks(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let split = with_mode(&pool, "split").await?;

    ReplHandler::new(split.clone()).retain().await?;

    assert_eq!(counts(&pool).await?, (5, 1, 2, 3));
    let reasons: Vec<String> =
        sqlx::query_scalar("SELECT peer || ': ' || reason FROM scd_retention_guard()")
            .fetch_all(&split)
            .await?;
    assert!(
        reasons.contains(&"subscriptions: 0 of 2 present".to_owned()),
        "{reasons:?}"
    );
    assert!(
        reasons.contains(&"main: no resend report".to_owned()),
        "{reasons:?}"
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn replay_runs_the_dead_command_again_and_drops_the_letter(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE OR REPLACE FUNCTION scd_cmd_track_edit(args jsonb) RETURNS jsonb LANGUAGE plpgsql AS $f$
         BEGIN
             UPDATE tracks SET title = args ->> 'title' WHERE urn = args ->> 'urn';
             IF NOT FOUND THEN
                 RAISE EXCEPTION 'no track' USING ERRCODE = 'check_violation';
             END IF;
             RETURN '{\"touched\": []}'::jsonb;
         END $f$",
    )
    .execute(&pool)
    .await?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO cmd_dead (node, id, kind, args, sqlstate, message)
         VALUES ('main', $1, 'track.edit', '{\"urn\": \"urn:t:1\", \"title\": \"Replayed\"}', '23514', 'no track')",
    )
    .bind(id)
    .execute(&pool)
    .await?;
    let handler = ReplHandler::new(pool.clone());
    let request = ReplReplayPayload {
        node: Some("main".to_owned()),
        id: Some(id),
    };

    let refused = handler.replay(request.clone()).await;
    assert!(
        refused.is_err(),
        "a replay that fails again is reported for the one letter asked for"
    );
    let still: i64 = sqlx::query_scalar("SELECT count(*) FROM cmd_dead WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(still, 1);

    sqlx::query(
        "INSERT INTO tracks (id, urn, sc_track_id, title, title_normalized, duration_ms)
         VALUES (gen_random_uuid(), 'urn:t:1', '1', 'Old', 'old', 1000)",
    )
    .execute(&pool)
    .await?;
    handler.replay(request).await?;

    let title: String = sqlx::query_scalar("SELECT title FROM tracks WHERE urn = 'urn:t:1'")
        .fetch_one(&pool)
        .await?;
    assert_eq!(title, "Replayed");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM cmd_dead")
        .fetch_one(&pool)
        .await?;
    assert_eq!(left, 0);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn job_status_follows_the_queue_and_the_reconcile_repairs_gaps(
    pool: PgPool,
) -> anyhow::Result<()> {
    let split = with_mode(&pool, "split").await?;
    sqlx::raw_sql(
        "INSERT INTO background_jobs (id, kind, lane, dedup_key, payload) VALUES
             (gen_random_uuid(), 'catalog.refresh', 'core_bulk', 'k1', '{}'),
             (gen_random_uuid(), 'crawl.artist', 'core_bulk', 'k2', '{}');
         UPDATE background_jobs SET lease_id = gen_random_uuid(), lease_generation = 1, leased_by = 'w',
                lease_expires_at = now() + interval '1 minute' WHERE dedup_key = 'k1'",
    )
    .execute(&split)
    .await?;
    let seen: Vec<(String, String)> =
        sqlx::query_as("SELECT dedup_key, state FROM job_status ORDER BY dedup_key")
            .fetch_all(&split)
            .await?;
    assert_eq!(seen, vec![("k1".to_owned(), "running".to_owned())]);

    sqlx::raw_sql(
        "ALTER TABLE background_jobs DISABLE TRIGGER zz_job_status_insert;
         ALTER TABLE background_jobs DISABLE TRIGGER zz_job_status_delete;
         INSERT INTO background_jobs (id, kind, lane, dedup_key, payload)
             VALUES (gen_random_uuid(), 'catalog.collection', 'core_bulk', 'k3', '{}');
         DELETE FROM background_jobs WHERE dedup_key = 'k1';
         ALTER TABLE background_jobs ENABLE TRIGGER zz_job_status_insert;
         ALTER TABLE background_jobs ENABLE TRIGGER zz_job_status_delete",
    )
    .execute(&split)
    .await?;

    ReplHandler::new(split.clone()).retain().await?;

    let seen: Vec<(String, String)> =
        sqlx::query_as("SELECT dedup_key, state FROM job_status ORDER BY dedup_key")
            .fetch_all(&split)
            .await?;
    assert_eq!(seen, vec![("k3".to_owned(), "queued".to_owned())]);
    Ok(())
}
