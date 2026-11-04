use std::collections::BTreeMap;

use sqlx::PgPool;

use super::{
    AUDIO_INDEX_QUARANTINE_SECONDS, QUALITY_UPGRADE_COOLDOWN_SECONDS, StarTiersHandler, Tier,
    mark_liked_tracks, of_tracks,
};

const ENRICH_MAX_ATTEMPTS: i16 = 5;
const FULL_PASS: &str = include_str!("refresh_due_full_pass.sql");

type Flags = (String, bool, bool, bool, bool, bool, Option<String>);

async fn seed_star_library(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO subscriptions (user_urn, exp_date) VALUES
             ('7', extract(epoch FROM now())::bigint + 3600),
             ('8', extract(epoch FROM now())::bigint - 3600);
         INSERT INTO user_likes_tracks (user_id, sc_track_id) VALUES
             ('7', '101'),
             ('soundcloud:users:7', '102'),
             ('8', '103');
         INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state) VALUES ('7', '109', false);
         INSERT INTO listening_history (soundcloud_user_id, sc_track_id, title, artist_name, duration, played_at)
         VALUES ('7', '104', 'Recent', 'Artist', 1000, (now() AT TIME ZONE 'UTC') - interval '1 day'),
                ('7', '105', 'Old', 'Artist', 1000, (now() AT TIME ZONE 'UTC') - interval '90 days'),
                ('7', '101', 'Liked', 'Artist', 1000, (now() AT TIME ZONE 'UTC'));
         INSERT INTO playlists (urn, sc_playlist_id, title, title_normalized, owner_sc_user_id)
         VALUES ('soundcloud:playlists:1', '1', 'Mine', 'mine', '7');
         INSERT INTO user_owned_playlists (user_id, playlist_urn) VALUES ('7', 'soundcloud:playlists:1');
         INSERT INTO playlist_track_projection (playlist_urn, position, sc_track_id)
         VALUES ('soundcloud:playlists:1', 0, '106');
         INSERT INTO star_track_tiers (sc_track_id, tier) VALUES ('103', 1), ('106', 1);",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn tiers(pool: &PgPool) -> anyhow::Result<BTreeMap<String, i16>> {
    let rows: Vec<(String, i16)> = sqlx::query_as("SELECT sc_track_id, tier FROM star_track_tiers")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().collect())
}

fn expected(pairs: &[(&str, i16)]) -> BTreeMap<String, i16> {
    pairs
        .iter()
        .map(|(id, tier)| ((*id).to_owned(), *tier))
        .collect()
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_refresh_follows_the_current_star_users_only(pool: PgPool) -> anyhow::Result<()> {
    seed_star_library(&pool).await?;

    StarTiersHandler::new(pool.clone(), ENRICH_MAX_ATTEMPTS)
        .refresh()
        .await?;

    assert_eq!(
        tiers(&pool).await?,
        expected(&[("101", 1), ("102", 1), ("104", 3), ("106", 3)])
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_refresh_marks_which_pipelines_still_wait_for_each_star_track(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_star_library(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms, storage_state, s3_verified_at
         ) VALUES
             ('101', 'soundcloud:tracks:101', 'Stored', 'stored', 1000, 'ok', now()),
             ('102', 'soundcloud:tracks:102', 'Indexed', 'indexed', 1000, 'ok', now());
         UPDATE tracks SET index_state = 'ok' WHERE sc_track_id = '102';
         UPDATE lyrics_lookup_state SET next_run_at = now() + interval '30 days'
         WHERE sc_track_id = '102';
         INSERT INTO lyrics_cache (sc_track_id, plain_text, source)
         VALUES ('102', 'a lyrics text long enough to be embedded later', 'genius');",
    )
    .execute(&pool)
    .await?;

    StarTiersHandler::new(pool.clone(), ENRICH_MAX_ATTEMPTS)
        .refresh()
        .await?;

    let due: Vec<(String, bool, bool, bool, bool)> = sqlx::query_as(
        "SELECT sc_track_id, audio_due, lookup_due, embed_due, align_due
         FROM star_track_tiers
         WHERE sc_track_id IN ('101', '102', '104')
         ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        due,
        [
            ("101".to_owned(), true, true, false, false),
            ("102".to_owned(), false, false, true, false),
            ("104".to_owned(), false, false, false, false),
        ]
    );
    Ok(())
}

async fn seed_uploaded_star_tracks(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO subscriptions (user_urn, exp_date)
         VALUES ('7', extract(epoch FROM now())::bigint + 3600);
         INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms, storage_state, s3_verified_at
         )
         SELECT id, 'soundcloud:tracks:' || id, 'T' || id, 't' || id, 1000, 'ok', now()
         FROM unnest(ARRAY['301', '302', '303', '304', '305', '306']) AS id;
         INSERT INTO user_likes_tracks (user_id, sc_track_id)
         SELECT '7', id FROM unnest(ARRAY['301', '302', '303', '304', '305', '306']) AS id;
         INSERT INTO storage_event_state (
             sc_track_id, stream, stream_sequence, event_published_at, uploaded_generation
         )
         SELECT id, 'storage', ordinal, now(), 2
         FROM unnest(ARRAY['301', '302', '303', '304', '305', '306']) WITH ORDINALITY AS uploads(id, ordinal);
         INSERT INTO audio_index_wire_state (
             sc_track_id, status, upload_generation, dispatched_at, outcome_status, outcome_reason
         ) VALUES
             ('301', 'terminal', 2, now() - interval '1 day', 'ok', 'indexed'),
             ('302', 'terminal', 1, now() - interval '1 day', 'ok', 'indexed'),
             ('303', 'pending', 2, now(), NULL, NULL);
         INSERT INTO lyrics_cache (sc_track_id, plain_text, synced_lrc, source) VALUES
             ('304', 'plain lyrics waiting for alignment', NULL, 'genius'),
             ('305', 'plain lyrics waiting for a new upload', NULL, 'genius'),
             ('306', 'plain lyrics already synced', '[00:01.00] synced', 'lrclib');
         UPDATE tracks SET transcribe_state = 'quarantined' WHERE sc_track_id = '305';
         INSERT INTO transcription_wire_state (
             sc_track_id, status, upload_generation, dispatched_at, quarantine_reason
         ) VALUES ('305', 'quarantined', 1, now() - interval '1 day', 'new_upload_during_pending');",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_refresh_marks_audio_and_alignment_by_the_wire_state_of_the_current_upload(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_uploaded_star_tracks(&pool).await?;

    StarTiersHandler::new(pool.clone(), ENRICH_MAX_ATTEMPTS)
        .refresh()
        .await?;

    let due: Vec<(String, bool, bool, bool)> = sqlx::query_as(
        "SELECT sc_track_id, audio_due, embed_due, align_due
         FROM star_track_tiers
         ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        due,
        [
            ("301".to_owned(), false, false, false),
            ("302".to_owned(), true, false, false),
            ("303".to_owned(), false, false, false),
            ("304".to_owned(), true, true, true),
            ("305".to_owned(), true, true, true),
            ("306".to_owned(), true, false, false),
        ]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_lapsed_star_user_drops_out_at_the_next_refresh(pool: PgPool) -> anyhow::Result<()> {
    seed_star_library(&pool).await?;
    let handler = StarTiersHandler::new(pool.clone(), ENRICH_MAX_ATTEMPTS);
    handler.refresh().await?;

    sqlx::query("UPDATE subscriptions SET exp_date = 0 WHERE user_urn = '7'")
        .execute(&pool)
        .await?;
    handler.refresh().await?;

    assert!(tiers(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_star_like_is_marked_at_once_and_other_likes_are_not(pool: PgPool) -> anyhow::Result<()> {
    seed_star_library(&pool).await?;
    let mut connection = pool.acquire().await?;

    mark_liked_tracks(&mut connection, "7", &["106".to_owned(), "201".to_owned()]).await?;
    mark_liked_tracks(&mut connection, "8", &["202".to_owned()]).await?;

    assert_eq!(
        tiers(&pool).await?,
        expected(&[("103", 1), ("106", 1), ("201", 1)])
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_track_tier_puts_star_likes_before_likes_before_star_listening(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, index_priority)
         VALUES ('1', 'soundcloud:tracks:1', 'A', 'a', 1000, 5),
                ('2', 'soundcloud:tracks:2', 'B', 'b', 1000, 1),
                ('3', 'soundcloud:tracks:3', 'C', 'c', 1000, 1),
                ('4', 'soundcloud:tracks:4', 'D', 'd', 1000, 5),
                ('5', 'soundcloud:tracks:5', 'Wave pick', 'wave pick', 1000, 0);
         INSERT INTO star_track_tiers (sc_track_id, tier) VALUES ('1', 1), ('3', 3), ('4', 3);",
    )
    .execute(&pool)
    .await?;
    let ids: Vec<String> = ["1", "2", "3", "4", "5", "6"].map(str::to_owned).to_vec();

    let found = of_tracks(&pool, &ids).await?;

    assert_eq!(found.get("1"), Some(&Tier::StarLiked));
    assert_eq!(found.get("2"), Some(&Tier::Liked));
    assert_eq!(found.get("3"), Some(&Tier::Liked));
    assert_eq!(found.get("5"), Some(&Tier::Rest));
    assert_eq!(found.get("4"), Some(&Tier::StarOther));
    assert_eq!(found.get("6"), None);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_refresh_marks_star_tracks_claimable_for_artist_links_and_a_better_copy(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_star_library(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, storage_quality)
         VALUES ('101', 'soundcloud:tracks:101', 'Sq', 'sq', 1000, 'sq'),
                ('102', 'soundcloud:tracks:102', 'Hq tried', 'hq tried', 1000, 'hq'),
                ('104', 'soundcloud:tracks:104', 'Uq', 'uq', 1000, 'uq'),
                ('106', 'soundcloud:tracks:106', 'Hq', 'hq', 1000, 'hq');
         UPDATE tracks SET hq_upgrade_pending = true WHERE sc_track_id IN ('101', '102', '106');
         UPDATE tracks SET hq_upgrade_last_at = now() - interval '1 hour' WHERE sc_track_id = '102';
         UPDATE tracks SET enrich_state = 'done' WHERE sc_track_id = '102';
         UPDATE tracks SET enrich_state = 'failed', enrich_attempts = 5 WHERE sc_track_id = '104';
         UPDATE tracks
         SET enrich_state = 'failed', enrich_next_run_at = now() + interval '1 hour'
         WHERE sc_track_id = '106';",
    )
    .execute(&pool)
    .await?;

    StarTiersHandler::new(pool.clone(), ENRICH_MAX_ATTEMPTS)
        .refresh()
        .await?;

    let due: Vec<(String, bool, Option<String>, bool)> = sqlx::query_as(
        "SELECT sc_track_id, enrich_due, upgrade_from, upgrade_tried_at IS NOT NULL
         FROM star_track_tiers
         ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        due,
        [
            ("101".to_owned(), true, Some("sq".to_owned()), false),
            ("102".to_owned(), false, None, true),
            ("104".to_owned(), false, None, false),
            ("106".to_owned(), false, Some("hq".to_owned()), false),
        ]
    );
    Ok(())
}

async fn flags(pool: &PgPool) -> anyhow::Result<Vec<Flags>> {
    Ok(sqlx::query_as(
        "SELECT sc_track_id, audio_due, lookup_due, embed_due, align_due, enrich_due, upgrade_from
         FROM star_track_tiers
         ORDER BY sc_track_id",
    )
    .fetch_all(pool)
    .await?)
}

async fn sweep(pool: &PgPool) -> anyhow::Result<(i64, i64)> {
    let due = sqlx::query_file!(
        "queries/star_priority/refresh_due.sql",
        AUDIO_INDEX_QUARANTINE_SECONDS,
        QUALITY_UPGRADE_COOLDOWN_SECONDS,
        ENRICH_MAX_ATTEMPTS,
        true
    )
    .fetch_one(pool)
    .await?;
    Ok((due.changed, due.missed))
}

async fn marks(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(
        sqlx::query_scalar("SELECT DISTINCT sc_track_id FROM star_due_marks ORDER BY sc_track_id")
            .fetch_all(pool)
            .await?,
    )
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn writes_after_a_sweep_reach_the_flags_through_their_marks(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_uploaded_star_tracks(&pool).await?;
    let handler = StarTiersHandler::new(pool.clone(), ENRICH_MAX_ATTEMPTS);
    handler.refresh().await?;
    let swept = flags(&pool).await?;

    sqlx::raw_sql(
        "DELETE FROM audio_index_wire_state WHERE sc_track_id = '301';
         UPDATE lyrics_lookup_state SET next_run_at = now() + interval '30 days'
         WHERE sc_track_id = '301';
         UPDATE audio_index_wire_state SET upload_generation = 2 WHERE sc_track_id = '302';
         UPDATE tracks SET enrich_next_run_at = now() + interval '1 day' WHERE sc_track_id = '302';
         UPDATE audio_index_wire_state SET dispatched_at = now() - interval '7 days'
         WHERE sc_track_id = '303';
         UPDATE tracks SET hq_upgrade_pending = true, storage_quality = 'sq'
         WHERE sc_track_id = '303';
         INSERT INTO transcription_wire_state (sc_track_id, status, upload_generation, dispatched_at)
         VALUES ('304', 'pending', 2, now());
         UPDATE lyrics_cache SET embedded_at = now() WHERE sc_track_id = '304';
         UPDATE storage_event_state SET uploaded_generation = 1 WHERE sc_track_id = '305';
         UPDATE lyrics_cache SET synced_lrc = NULL WHERE sc_track_id = '306';",
    )
    .execute(&pool)
    .await?;
    assert_eq!(
        marks(&pool).await?,
        ["301", "302", "303", "304", "305", "306"]
    );

    handler.refresh().await?;
    let dirty = flags(&pool).await?;

    assert!(marks(&pool).await?.is_empty());
    assert_eq!(dirty.len(), swept.len());
    assert!(
        dirty
            .iter()
            .zip(&swept)
            .all(|(after, before)| after != before)
    );
    assert_eq!(sweep(&pool).await?, (0, 0));
    assert_eq!(flags(&pool).await?, dirty);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn the_sweep_fixes_and_counts_a_change_no_mark_reached(pool: PgPool) -> anyhow::Result<()> {
    seed_uploaded_star_tracks(&pool).await?;
    let handler = StarTiersHandler::new(pool.clone(), ENRICH_MAX_ATTEMPTS);
    handler.refresh().await?;
    sqlx::raw_sql(
        "ALTER TABLE audio_index_wire_state DISABLE TRIGGER star_due_mark_update;
         UPDATE audio_index_wire_state SET upload_generation = 2 WHERE sc_track_id = '302';",
    )
    .execute(&pool)
    .await?;

    handler.refresh().await?;
    let stale: bool =
        sqlx::query_scalar("SELECT audio_due FROM star_track_tiers WHERE sc_track_id = '302'")
            .fetch_one(&pool)
            .await?;

    assert!(stale);
    assert_eq!(sweep(&pool).await?, (1, 1));
    let fixed: bool =
        sqlx::query_scalar("SELECT audio_due FROM star_track_tiers WHERE sc_track_id = '302'")
            .fetch_one(&pool)
            .await?;
    assert!(!fixed);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_flag_that_turns_with_time_is_rechecked_without_a_mark(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_uploaded_star_tracks(&pool).await?;
    let handler = StarTiersHandler::new(pool.clone(), ENRICH_MAX_ATTEMPTS);
    handler.refresh().await?;
    sqlx::query(
        "UPDATE lyrics_lookup_state SET next_run_at = now() + interval '2 hours'
         WHERE sc_track_id = '301'",
    )
    .execute(&pool)
    .await?;
    handler.refresh().await?;
    let waiting: (bool, bool) = sqlx::query_as(
        "SELECT star.lookup_due, star.recheck_at = queue.next_run_at
         FROM star_track_tiers AS star
         JOIN lyrics_lookup_state AS state ON state.sc_track_id = star.sc_track_id
         JOIN lyrics_lookup_queue AS queue ON queue.track_id = state.track_id
         WHERE star.sc_track_id = '301'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(waiting, (false, true));

    sqlx::raw_sql(
        "ALTER TABLE lyrics_lookup_queue DISABLE TRIGGER star_due_mark_update;
         UPDATE lyrics_lookup_state SET next_run_at = now() - interval '1 minute'
         WHERE sc_track_id = '301';
         UPDATE star_track_tiers SET recheck_at = now() - interval '1 minute'
         WHERE sc_track_id = '301';",
    )
    .execute(&pool)
    .await?;
    handler.refresh().await?;

    let due: (bool, bool) = sqlx::query_as(
        "SELECT lookup_due, recheck_at IS NULL FROM star_track_tiers WHERE sc_track_id = '301'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(due, (true, true));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn writes_mark_star_tracks_only_while_a_refresh_watches(pool: PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
         VALUES ('1', 'soundcloud:tracks:1', 'Star', 'star', 1000),
                ('2', 'soundcloud:tracks:2', 'Other', 'other', 1000);
         INSERT INTO star_track_tiers (sc_track_id, tier) VALUES ('1', 1);
         UPDATE tracks SET index_state = 'ok';
         INSERT INTO star_due_watch (singleton, watched_at) VALUES (true, now() - interval '2 days');
         UPDATE tracks SET index_state = 'pending';",
    )
    .execute(&pool)
    .await?;
    assert!(marks(&pool).await?.is_empty());

    sqlx::raw_sql(
        "UPDATE star_due_watch SET watched_at = now();
         UPDATE tracks SET index_state = 'ok';
         UPDATE tracks SET likes_count_sc = 10;",
    )
    .execute(&pool)
    .await?;

    let marked: Vec<String> = sqlx::query_scalar("SELECT sc_track_id FROM star_due_marks")
        .fetch_all(&pool)
        .await?;
    assert_eq!(marked, ["1"]);
    Ok(())
}

async fn apply_index_state(pool: &PgPool, settings: &str, state: &str) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::raw_sql(settings).execute(&mut *tx).await?;
    sqlx::query("UPDATE tracks SET index_state = $1 WHERE sc_track_id = '1'")
        .bind(state)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_applied_change_marks_on_core_in_split_and_waits_for_the_sweep_in_legacy(
    pool: PgPool,
) -> anyhow::Result<()> {
    let triggers: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT tgrelid::regclass::text, tgname::text, tgenabled::text
         FROM pg_trigger
         WHERE tgname IN ('star_due_mark_update', 'star_due_mark_row')
         ORDER BY tgrelid::regclass::text COLLATE \"C\", tgname::text COLLATE \"C\"",
    )
    .fetch_all(&pool)
    .await?;

    let tables = [
        "audio_index_wire_state",
        "lyrics_cache",
        "lyrics_embedding_wire_state",
        "lyrics_lookup_queue",
        "lyrics_lookup_state",
        "storage_event_state",
        "tracks",
        "transcription_wire_state",
    ];
    let expected: Vec<(String, String, String)> = tables
        .iter()
        .flat_map(|table| {
            ["star_due_mark_row", "star_due_mark_update"]
                .map(|name| ((*table).to_owned(), name.to_owned(), "A".to_owned()))
        })
        .collect();
    assert_eq!(triggers, expected);

    sqlx::raw_sql(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
         VALUES ('1', 'soundcloud:tracks:1', 'Star', 'star', 1000);
         INSERT INTO star_track_tiers (sc_track_id, tier) VALUES ('1', 1);
         INSERT INTO star_due_watch (singleton, watched_at) VALUES (true, now());",
    )
    .execute(&pool)
    .await?;

    apply_index_state(&pool, "SET LOCAL session_replication_role = replica", "ok").await?;
    assert!(marks(&pool).await?.is_empty());

    apply_index_state(
        &pool,
        "SET LOCAL scd.mode = 'split';
         SET LOCAL scd.role = 'core';
         SET LOCAL session_replication_role = replica",
        "pending",
    )
    .await?;
    assert_eq!(marks(&pool).await?, ["1"]);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_queue_change_of_the_due_time_or_the_claim_marks_the_star_track(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_uploaded_star_tracks(&pool).await?;
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
         VALUES ('399', 'soundcloud:tracks:399', 'Other', 'other', 1000)",
    )
    .execute(&pool)
    .await?;
    let handler = StarTiersHandler::new(pool.clone(), ENRICH_MAX_ATTEMPTS);
    handler.refresh().await?;
    let rounds: [(&str, &[&str], bool); 4] = [
        (
            "UPDATE lyrics_lookup_queue AS queue
             SET claim_job_id = gen_random_uuid(),
                 claim_job_generation = 1,
                 claim_job_lease_id = gen_random_uuid(),
                 claim_state_generation = state.generation,
                 claim_expires_at = now() + interval '15 minutes',
                 attempts = queue.attempts + 1,
                 last_attempt_at = now()
             FROM lyrics_lookup_state AS state
             WHERE state.track_id = queue.track_id
               AND state.sc_track_id IN ('301', '399')",
            &["301"],
            false,
        ),
        (
            "UPDATE lyrics_lookup_queue AS queue
             SET next_run_at = now() + interval '1 hour',
                 claim_job_id = NULL,
                 claim_job_generation = NULL,
                 claim_job_lease_id = NULL,
                 claim_state_generation = NULL,
                 claim_expires_at = NULL
             FROM lyrics_lookup_state AS state
             WHERE state.track_id = queue.track_id
               AND state.sc_track_id IN ('301', '399')",
            &["301"],
            false,
        ),
        (
            "UPDATE lyrics_lookup_queue AS queue
             SET next_run_at = now() - interval '1 minute'
             FROM lyrics_lookup_state AS state
             WHERE state.track_id = queue.track_id
               AND state.sc_track_id IN ('301', '399')",
            &["301"],
            true,
        ),
        (
            "UPDATE lyrics_lookup_queue
             SET attempts = attempts + 1, last_attempt_at = now(), priority = 0",
            &[],
            true,
        ),
    ];

    for (write, marked, due) in rounds {
        sqlx::query(write).execute(&pool).await?;
        assert_eq!(marks(&pool).await?, marked, "{write}");
        handler.refresh().await?;
        let lookup_due: bool =
            sqlx::query_scalar("SELECT lookup_due FROM star_track_tiers WHERE sc_track_id = '301'")
                .fetch_one(&pool)
                .await?;
        assert_eq!(lookup_due, due, "{write}");
        assert_eq!(sweep(&pool).await?, (0, 0), "{write}");
    }
    Ok(())
}

async fn full_pass(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(FULL_PASS)
        .bind(AUDIO_INDEX_QUARANTINE_SECONDS)
        .bind(QUALITY_UPGRADE_COOLDOWN_SECONDS)
        .bind(ENRICH_MAX_ATTEMPTS)
        .fetch_one(pool)
        .await?)
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn every_dirty_pass_leaves_the_flags_of_the_old_full_pass(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_uploaded_star_tracks(&pool).await?;
    sqlx::raw_sql(
        "UPDATE tracks SET index_state = 'ok' WHERE sc_track_id = '301';
         UPDATE tracks SET enrich_state = 'failed', enrich_attempts = 5 WHERE sc_track_id = '302';
         UPDATE tracks SET enrich_state = 'failed', enrich_next_run_at = now() + interval '1 hour'
         WHERE sc_track_id = '303';
         UPDATE tracks SET hq_upgrade_pending = true, storage_quality = 'hq',
                           hq_upgrade_last_at = now() - interval '1 hour'
         WHERE sc_track_id = '304';
         UPDATE lyrics_lookup_state SET next_run_at = now() + interval '30 days'
         WHERE sc_track_id = '305';",
    )
    .execute(&pool)
    .await?;
    let handler = StarTiersHandler::new(pool.clone(), ENRICH_MAX_ATTEMPTS);
    handler.refresh().await?;
    assert_eq!(full_pass(&pool).await?, 0);

    let rounds = [
        "UPDATE lyrics_lookup_queue AS queue
         SET claim_job_id = gen_random_uuid(),
             claim_job_generation = 1,
             claim_job_lease_id = gen_random_uuid(),
             claim_state_generation = state.generation,
             claim_expires_at = now() + interval '15 minutes',
             attempts = queue.attempts + 1,
             last_attempt_at = now()
         FROM lyrics_lookup_state AS state
         WHERE state.track_id = queue.track_id
           AND state.sc_track_id IN ('301', '302')",
        "UPDATE lyrics_lookup_queue AS queue
         SET next_run_at = CASE state.sc_track_id
                 WHEN '301' THEN now() + interval '1 hour'
                 ELSE now() - interval '1 minute'
             END,
             claim_job_id = NULL,
             claim_job_generation = NULL,
             claim_job_lease_id = NULL,
             claim_state_generation = NULL,
             claim_expires_at = NULL
         FROM lyrics_lookup_state AS state
         WHERE state.track_id = queue.track_id
           AND state.sc_track_id IN ('301', '302')",
        "UPDATE lyrics_lookup_state SET next_run_at = now() + interval '1 day'
         WHERE sc_track_id = '303'",
        "UPDATE lyrics_lookup_state SET next_run_at = now() - interval '1 minute'
         WHERE sc_track_id = '305'",
        "DELETE FROM lyrics_lookup_state WHERE sc_track_id = '304'",
        "UPDATE tracks SET enrich_next_run_at = now() - interval '1 minute' WHERE sc_track_id = '303'",
        "UPDATE tracks SET hq_upgrade_pending = true, storage_quality = 'sq' WHERE sc_track_id = '306'",
        "UPDATE audio_index_wire_state
         SET status = 'terminal', upload_generation = 1, outcome_status = 'failed',
             outcome_reason = 'worker_lost'
         WHERE sc_track_id = '303'",
        "UPDATE lyrics_cache SET embedded_at = now() WHERE sc_track_id = '305'",
        "UPDATE storage_event_state SET uploaded_generation = 3 WHERE sc_track_id = '301'",
        "INSERT INTO tracks (
             sc_track_id, urn, title, title_normalized, duration_ms, storage_state, s3_verified_at
         ) VALUES ('307', 'soundcloud:tracks:307', 'T307', 't307', 1000, 'ok', now());
         INSERT INTO user_likes_tracks (user_id, sc_track_id) VALUES ('7', '307')",
        "UPDATE lyrics_lookup_queue SET attempts = attempts + 1, last_attempt_at = now();
         UPDATE tracks SET likes_count_sc = 10",
    ];
    for round in rounds {
        sqlx::raw_sql(round).execute(&pool).await?;
        handler.refresh().await?;
        assert_eq!(full_pass(&pool).await?, 0, "{round}");
    }

    let before = flags(&pool).await?;
    assert_eq!(sweep(&pool).await?, (0, 0));
    assert_eq!(full_pass(&pool).await?, 0);
    assert_eq!(flags(&pool).await?, before);
    assert_eq!(before.len(), 7);
    Ok(())
}
