use serde_json::Value;

use super::*;

fn switches(enabled: bool) -> WorkerDispatchConfig {
    WorkerDispatchConfig {
        embed_lyrics: enabled,
        index_audio: false,
        transcribe: enabled,
        lyrics_align_rejected_retry_days: 30,
    }
}

fn reaper(pool: &PgPool) -> LyricsReaper {
    LyricsReaper::new(pool.clone(), switches(true))
}

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0057_background_jobs.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(
        "CREATE TABLE tracks (
             sc_track_id text PRIMARY KEY,
             storage_state varchar(16) NOT NULL,
             needs_duration_resolve boolean NOT NULL DEFAULT false,
             transcribe_state varchar(16),
             transcribe_at timestamptz,
             created_at timestamptz NOT NULL DEFAULT now(),
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE storage_event_state (
             sc_track_id text PRIMARY KEY REFERENCES tracks(sc_track_id) ON DELETE CASCADE,
             stream varchar(128) NOT NULL,
             stream_sequence bigint NOT NULL,
             event_published_at timestamptz NOT NULL,
             uploaded_generation bigint NOT NULL,
             transcription_generation bigint,
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE transcription_wire_state (
             sc_track_id text PRIMARY KEY,
             status varchar(16) NOT NULL,
             upload_generation bigint,
             dispatched_at timestamptz,
             completed_at timestamptz,
             quarantine_reason varchar(64),
             result_stream_sequence bigint,
             result_published_at timestamptz,
             attempt bigint NOT NULL DEFAULT 1,
             reopen_count integer NOT NULL DEFAULT 0,
             result_rank smallint,
             reason varchar(32),
             sync_version varchar(128),
             reopened_for_sync_version varchar(128),
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE transcription_sync_versions (
             sync_version varchar(128) PRIMARY KEY,
             first_seen_at timestamptz NOT NULL DEFAULT now(),
             last_seen_at timestamptz NOT NULL DEFAULT now()
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
             content_generation bigint NOT NULL DEFAULT 1,
             created_at timestamp NOT NULL DEFAULT now()
         );
         CREATE TABLE lyrics_embedding_wire_state (
             sc_track_id text PRIMARY KEY,
             status varchar(16) NOT NULL,
             lyrics_created_at timestamp,
             lyrics_content_generation bigint,
             first_publish_attempt_at timestamptz,
             completed_at timestamptz,
             quarantine_reason varchar(64),
             result_consumer varchar(96),
             result_lease_id uuid,
             result_lease_expires_at timestamptz,
             updated_at timestamptz NOT NULL DEFAULT now()
         );",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_stored_track(pool: &PgPool, id: &str, generation: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (
             sc_track_id, storage_state, needs_duration_resolve, created_at
         ) VALUES ($1, 'ok', false, now() - interval '1 hour')",
    )
    .bind(id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO storage_event_state (
             sc_track_id, stream, stream_sequence, event_published_at,
             uploaded_generation, updated_at
         ) VALUES ($1, 'STORAGE_EVENTS', $2, now() - interval '1 hour',
                   $2, now() - interval '1 hour')",
    )
    .bind(id)
    .bind(generation)
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_lyrics(pool: &PgPool, id: &str, text: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, plain_text, source, created_at)
         VALUES ($1, $2, 'genius', (now() AT TIME ZONE 'UTC') - interval '1 hour')",
    )
    .bind(id)
    .bind(text)
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn transcription_reaper_enqueues_only_current_never_dispatched_work(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_stored_track(&pool, "41", 3).await?;
    seed_lyrics(&pool, "41", "plain lyrics waiting for alignment").await?;
    seed_stored_track(&pool, "42", 4).await?;
    seed_stored_track(&pool, "43", 5).await?;
    sqlx::query("UPDATE tracks SET transcribe_state = 'done' WHERE sc_track_id = '43'")
        .execute(&pool)
        .await?;
    seed_stored_track(&pool, "44", 6).await?;
    sqlx::query(
        "INSERT INTO transcription_wire_state (
             sc_track_id, status, upload_generation, dispatched_at
         ) VALUES ('44', 'quarantined', 6, now() - interval '1 hour')",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO tracks (
             sc_track_id, storage_state, needs_duration_resolve, created_at
         ) VALUES ('45', 'ok', false, now() - interval '1 hour')",
    )
    .execute(&pool)
    .await?;

    let reaper = reaper(&pool);
    reaper.reap_transcriptions().await?;
    reaper.reap_transcriptions().await?;

    let jobs = sqlx::query_as::<_, (String, i64, Value)>(
        "SELECT dedup_key, generation, payload
         FROM background_jobs
         WHERE kind = 'lyrics.dispatch_transcription'
         ORDER BY dedup_key",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(jobs.len(), 1);
    assert_eq!((jobs[0].0.as_str(), jobs[0].1), ("41", 1));
    assert_eq!(jobs[0].2["payload"]["uploaded_generation"], 3);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn stale_transcription_is_quarantined_without_redispatch(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_stored_track(&pool, "42", 1).await?;
    sqlx::query(
        "UPDATE tracks
         SET transcribe_state = 'pending', transcribe_at = now() - interval '49 hours'
         WHERE sc_track_id = '42'",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "UPDATE storage_event_state
         SET transcription_generation = 1
         WHERE sc_track_id = '42'",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO transcription_wire_state (
             sc_track_id, status, upload_generation, dispatched_at
         ) VALUES
             ('42', 'pending', 1, now() - interval '49 hours'),
             ('99', 'pending', 1, now() - interval '49 hours')",
    )
    .execute(&pool)
    .await?;

    reaper(&pool).reap_transcriptions().await?;

    let state = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT track.transcribe_state, wire.status, wire.quarantine_reason
         FROM tracks AS track
         JOIN transcription_wire_state AS wire USING (sc_track_id)
         WHERE track.sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        state,
        (
            "quarantined".to_owned(),
            "quarantined".to_owned(),
            Some("result_timeout".to_owned()),
        )
    );
    assert_eq!(jobs, 0);
    let orphan = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, quarantine_reason
         FROM transcription_wire_state
         WHERE sc_track_id = '99'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        orphan,
        (
            "quarantined".to_owned(),
            Some("track_missing_after_result_timeout".to_owned()),
        )
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn old_orphaned_lyrics_are_quarantined_without_gpu_work(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    sqlx::query(
        "INSERT INTO lyrics_cache (
             sc_track_id, plain_text, source, created_at
         ) VALUES (
             '42', 'orphaned lyrics that must not stay in the recovery index',
             'genius', (now() AT TIME ZONE 'UTC') - interval '49 hours'
         )",
    )
    .execute(&pool)
    .await?;

    reaper(&pool).reap_embeddings().await?;

    let state: Option<String> =
        sqlx::query_scalar("SELECT embedding_state FROM lyrics_cache WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(state.as_deref(), Some("quarantined"));
    assert_eq!(jobs, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn stale_embedding_requests_are_quarantined_without_redispatch(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_stored_track(&pool, "42", 1).await?;
    seed_lyrics(
        &pool,
        "42",
        "lyrics whose one embedding request never produced a result",
    )
    .await?;
    sqlx::raw_sql(
        "UPDATE lyrics_cache
         SET embedding_state = 'dispatched'
         WHERE sc_track_id = '42';
         INSERT INTO lyrics_embedding_wire_state (
             sc_track_id, status, first_publish_attempt_at
         ) VALUES
             ('42', 'pending', now() - interval '49 hours'),
             ('99', 'pending', now() - interval '49 hours');
         UPDATE lyrics_embedding_wire_state AS wire
         SET lyrics_created_at = cache.created_at,
             lyrics_content_generation = cache.content_generation
         FROM lyrics_cache AS cache
         WHERE cache.sc_track_id = wire.sc_track_id",
    )
    .execute(&pool)
    .await?;

    reaper(&pool).reap_embeddings().await?;

    let states = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT wire.sc_track_id, wire.status, wire.quarantine_reason
         FROM lyrics_embedding_wire_state AS wire
         ORDER BY wire.sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        states,
        vec![
            (
                "42".to_owned(),
                "quarantined".to_owned(),
                Some("result_timeout".to_owned()),
            ),
            (
                "99".to_owned(),
                "quarantined".to_owned(),
                Some("lyrics_missing_after_result_timeout".to_owned()),
            ),
        ]
    );
    let cache_state: Option<String> =
        sqlx::query_scalar("SELECT embedding_state FROM lyrics_cache WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(cache_state.as_deref(), Some("quarantined"));
    assert_eq!(jobs, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_request_outlived_by_replaced_lyrics_times_out_and_the_new_lyrics_are_embedded(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_stored_track(&pool, "42", 1).await?;
    seed_lyrics(
        &pool,
        "42",
        "lyrics that were replaced while their embedding request was in flight",
    )
    .await?;
    sqlx::raw_sql(
        "INSERT INTO lyrics_embedding_wire_state (
             sc_track_id, status, first_publish_attempt_at,
             lyrics_created_at, lyrics_content_generation
         )
         SELECT sc_track_id, 'pending', now() - interval '49 hours', created_at, content_generation
         FROM lyrics_cache WHERE sc_track_id = '42';
         UPDATE lyrics_cache
         SET embedding_state = NULL, content_generation = content_generation + 1
         WHERE sc_track_id = '42';",
    )
    .execute(&pool)
    .await?;

    reaper(&pool).reap_embeddings().await?;

    let wire = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT status, quarantine_reason FROM lyrics_embedding_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let queued: Vec<String> =
        sqlx::query_scalar("SELECT dedup_key FROM background_jobs WHERE kind = 'lyrics.embed'")
            .fetch_all(&pool)
            .await?;
    let cache_state: Option<String> =
        sqlx::query_scalar("SELECT embedding_state FROM lyrics_cache WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        wire,
        (
            "quarantined".to_owned(),
            Some("lyrics_changed_before_apply".to_owned())
        )
    );
    assert_eq!(queued, vec!["42".to_owned()]);
    assert_eq!(cache_state.as_deref(), Some("queued"));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn embedding_reaper_waits_for_active_result_claim(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_stored_track(&pool, "42", 1).await?;
    seed_lyrics(
        &pool,
        "42",
        "lyrics whose result is currently being written to the vector store",
    )
    .await?;
    let lease_id = Uuid::now_v7();
    sqlx::query(
        "UPDATE lyrics_cache
         SET embedding_state = 'dispatched'
         WHERE sc_track_id = '42'",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO lyrics_embedding_wire_state (
             sc_track_id, status, first_publish_attempt_at,
             result_consumer, result_lease_id, result_lease_expires_at
         ) VALUES (
             '42', 'pending', now() - interval '49 hours',
             'backend-done-embed-lyrics', $1, now() + interval '1 minute'
         )",
    )
    .bind(lease_id)
    .execute(&pool)
    .await?;

    reaper(&pool).reap_embeddings().await?;
    let active: String = sqlx::query_scalar(
        "SELECT status FROM lyrics_embedding_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    sqlx::query(
        "UPDATE lyrics_embedding_wire_state
         SET result_lease_expires_at = now() - interval '1 second'
         WHERE sc_track_id = '42'",
    )
    .execute(&pool)
    .await?;
    reaper(&pool).reap_embeddings().await?;
    let recently_released: String = sqlx::query_scalar(
        "SELECT status FROM lyrics_embedding_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    sqlx::query(
        "UPDATE lyrics_embedding_wire_state
         SET updated_at = now() - interval '49 hours'
         WHERE sc_track_id = '42'",
    )
    .execute(&pool)
    .await?;
    reaper(&pool).reap_embeddings().await?;
    let expired = sqlx::query_as::<_, (String, Option<Uuid>)>(
        "SELECT status, result_lease_id
         FROM lyrics_embedding_wire_state
         WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;

    assert_eq!(active, "pending");
    assert_eq!(recently_released, "pending");
    assert_eq!(expired, ("quarantined".to_owned(), None));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn embedding_reaper_records_one_shot_state_before_enqueue(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_stored_track(&pool, "42", 1).await?;
    seed_lyrics(
        &pool,
        "42",
        "lyrics long enough to require a durable embedding job",
    )
    .await?;

    let first = reaper(&pool);
    let second = reaper(&pool);
    let (first_result, second_result) =
        tokio::join!(first.reap_embeddings(), second.reap_embeddings(),);
    first_result?;
    second_result?;
    first.reap_embeddings().await?;

    let state = sqlx::query_as::<_, (String, i64, i32)>(
        "SELECT lyrics.embedding_state, job.generation, job.attempts
         FROM lyrics_cache AS lyrics
         JOIN background_jobs AS job
           ON job.kind = 'lyrics.embed' AND job.dedup_key = lyrics.sc_track_id
         WHERE lyrics.sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(state, ("queued".to_owned(), 1, 0));
    let wire_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_embedding_wire_state")
        .fetch_one(&pool)
        .await?;
    assert_eq!(wire_rows, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn embedding_reaper_skips_short_terminal_and_already_queued_rows(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    for id in ["41", "42", "43"] {
        seed_stored_track(&pool, id, id.parse()?).await?;
    }
    seed_lyrics(&pool, "41", "short").await?;
    seed_lyrics(
        &pool,
        "42",
        "terminal lyrics that must never be dispatched another time",
    )
    .await?;
    seed_lyrics(
        &pool,
        "43",
        "queued lyrics that must retain the original background job",
    )
    .await?;
    sqlx::raw_sql(
        "INSERT INTO lyrics_embedding_wire_state (
             sc_track_id, status, completed_at
         ) VALUES ('42', 'done', now());
         UPDATE lyrics_cache
         SET embedding_state = 'done', embedded_at = now()
         WHERE sc_track_id = '42';",
    )
    .execute(&pool)
    .await?;
    let queue = JobRepository::new(pool.clone(), "test".to_owned());
    let existing = NewJob {
        id: Uuid::now_v7(),
        kind: JobKind::LyricsEmbed,
        dedup_key: Some("43".to_owned()),
        payload: serde_json::json!({ "original": true }),
        priority: 1,
        max_attempts: 3,
        available_at: Utc::now(),
    };
    queue.enqueue(&existing).await?;

    reaper(&pool).reap_embeddings().await?;

    let jobs = sqlx::query_as::<_, (String, Value, i64)>(
        "SELECT dedup_key, payload, generation
         FROM background_jobs
         ORDER BY dedup_key",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(jobs, vec![("43".to_owned(), existing.payload, 1)]);
    Ok(())
}

async fn seed_wire(
    pool: &PgPool,
    id: &str,
    status: &str,
    generation: i64,
    completed_hours_ago: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO transcription_wire_state (
             sc_track_id, status, upload_generation, attempt, dispatched_at, completed_at,
             reason, result_rank, sync_version
         ) VALUES (
             $1, $2, $3, 2, now() - interval '2 days',
             now() - $4 * interval '1 hour',
             CASE WHEN $2 = 'reopenable' THEN 'engine_restarted' ELSE 'low_confidence' END,
             CASE WHEN $2 = 'reopenable' THEN 1 ELSE 3 END,
             'new'
         )",
    )
    .bind(id)
    .bind(status)
    .bind(generation)
    .bind(completed_hours_ago)
    .execute(pool)
    .await?;
    Ok(())
}

async fn dispatched_generations(pool: &PgPool) -> anyhow::Result<Vec<(String, i64)>> {
    let jobs = sqlx::query_as::<_, (String, Value)>(
        "SELECT dedup_key, payload
         FROM background_jobs
         WHERE kind = 'lyrics.dispatch_transcription'
         ORDER BY dedup_key",
    )
    .fetch_all(pool)
    .await?;
    Ok(jobs
        .into_iter()
        .map(|(id, payload)| {
            let generation = payload["payload"]["uploaded_generation"]
                .as_i64()
                .unwrap_or_default();
            (id, generation)
        })
        .collect())
}

async fn wire_state(
    pool: &PgPool,
    id: &str,
) -> anyhow::Result<(String, i64, i32, Option<String>, Option<String>)> {
    Ok(sqlx::query_as(
        "SELECT status, attempt, reopen_count, reason, quarantine_reason
         FROM transcription_wire_state
         WHERE sc_track_id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrations = false)]
async fn switched_off_dispatch_enqueues_nothing_but_still_quarantines(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_stored_track(&pool, "41", 1).await?;
    seed_lyrics(
        &pool,
        "41",
        "plain lyrics waiting for alignment and embedding",
    )
    .await?;
    seed_stored_track(&pool, "42", 1).await?;
    seed_lyrics(&pool, "42", "plain lyrics of an interrupted alignment").await?;
    seed_wire(&pool, "42", "reopenable", 1, 7).await?;
    sqlx::query(
        "INSERT INTO transcription_wire_state (
             sc_track_id, status, upload_generation, dispatched_at
         ) VALUES ('43', 'pending', 1, now() - interval '26 hours')",
    )
    .execute(&pool)
    .await?;

    let reaper = LyricsReaper::new(pool.clone(), switches(false));
    reaper.reap_transcriptions().await?;
    reaper.reap_embeddings().await?;

    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    let orphan = wire_state(&pool, "43").await?;
    assert_eq!(jobs, 0);
    assert_eq!(wire_state(&pool, "42").await?.0, "reopenable");
    assert_eq!(orphan.0, "quarantined");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_pending_attempt_is_quarantined_only_after_the_lane_result_window(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    for id in ["41", "42"] {
        seed_stored_track(&pool, id, 1).await?;
    }
    sqlx::raw_sql(
        "UPDATE tracks SET transcribe_state = 'pending';
         INSERT INTO transcription_wire_state (
             sc_track_id, status, upload_generation, dispatched_at
         ) VALUES
             ('41', 'pending', 1, now() - interval '25 hours'),
             ('42', 'pending', 1, now() - interval '26 hours');",
    )
    .execute(&pool)
    .await?;

    reaper(&pool).reap_transcriptions().await?;

    let inside = wire_state(&pool, "41").await?;
    let outside = wire_state(&pool, "42").await?;
    assert_eq!(inside.0, "pending");
    assert_eq!(
        (outside.0, outside.4),
        ("quarantined".to_owned(), Some("result_timeout".to_owned()))
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_reopenable_attempt_is_dispatched_again_after_the_cooldown(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    for (id, generation) in [("41", 3), ("42", 4)] {
        seed_stored_track(&pool, id, generation).await?;
        seed_lyrics(&pool, id, "plain lyrics of an interrupted alignment").await?;
        seed_wire(&pool, id, "reopenable", generation, 0).await?;
    }
    sqlx::query(
        "UPDATE transcription_wire_state
         SET completed_at = now() - interval '7 hours'
         WHERE sc_track_id = '41'",
    )
    .execute(&pool)
    .await?;

    reaper(&pool).reap_transcriptions().await?;

    let track_state: Option<String> =
        sqlx::query_scalar("SELECT transcribe_state FROM tracks WHERE sc_track_id = '41'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        wire_state(&pool, "41").await?,
        ("pending".to_owned(), 3, 1, None, None)
    );
    assert_eq!(track_state.as_deref(), Some("pending"));
    assert_eq!(wire_state(&pool, "42").await?.0, "reopenable");
    assert_eq!(
        dispatched_generations(&pool).await?,
        vec![("41".to_owned(), 3)]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_reopenable_attempt_that_cannot_run_again_is_quarantined(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    for id in ["41", "42"] {
        seed_stored_track(&pool, id, 3).await?;
        seed_lyrics(&pool, id, "plain lyrics of an interrupted alignment").await?;
    }
    seed_wire(&pool, "41", "reopenable", 3, 7).await?;
    seed_wire(&pool, "42", "reopenable", 2, 7).await?;
    sqlx::query("UPDATE transcription_wire_state SET reopen_count = 7 WHERE sc_track_id = '41'")
        .execute(&pool)
        .await?;

    reaper(&pool).reap_transcriptions().await?;

    let exhausted = wire_state(&pool, "41").await?;
    let superseded = wire_state(&pool, "42").await?;
    assert_eq!(
        (exhausted.0, exhausted.4),
        (
            "quarantined".to_owned(),
            Some("reopen_attempts_exhausted".to_owned())
        )
    );
    assert_eq!(
        (superseded.0, superseded.4),
        (
            "quarantined".to_owned(),
            Some("reopen_superseded".to_owned())
        )
    );
    assert!(dispatched_generations(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn rejected_attempts_are_reevaluated_for_a_newer_sync_version_or_after_the_retry_window(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    for id in ["41", "42", "43", "44"] {
        seed_stored_track(&pool, id, 1).await?;
        seed_lyrics(&pool, id, "plain lyrics the gate once rejected").await?;
    }
    seed_wire(&pool, "41", "rejected", 1, 7).await?;
    seed_wire(&pool, "42", "rejected", 1, 7).await?;
    seed_wire(&pool, "43", "rejected", 1, 31 * 24).await?;
    seed_wire(&pool, "44", "rejected", 1, 1).await?;
    sqlx::raw_sql(
        "UPDATE transcription_wire_state
         SET sync_version = 'old'
         WHERE sc_track_id IN ('41', '44');
         INSERT INTO transcription_sync_versions (sync_version, first_seen_at) VALUES
             ('old', now() - interval '10 days'),
             ('new', now() - interval '1 day');",
    )
    .execute(&pool)
    .await?;

    reaper(&pool).reap_transcriptions().await?;

    assert_eq!(
        dispatched_generations(&pool).await?,
        vec![("41".to_owned(), 1), ("43".to_owned(), 1)]
    );
    assert_eq!(
        wire_state(&pool, "41").await?,
        ("pending".to_owned(), 3, 0, None, None)
    );
    assert_eq!(wire_state(&pool, "42").await?.0, "rejected");
    assert_eq!(wire_state(&pool, "44").await?.0, "rejected");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_transcription_superseded_by_a_new_upload_is_dispatched_for_it(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    for id in ["41", "42"] {
        seed_stored_track(&pool, id, 2).await?;
        seed_lyrics(&pool, id, "plain lyrics of an upgraded upload").await?;
    }
    sqlx::raw_sql(
        "UPDATE tracks SET transcribe_state = 'quarantined';
         UPDATE storage_event_state SET transcription_generation = 1;
         INSERT INTO transcription_wire_state (
             sc_track_id, status, upload_generation, dispatched_at, completed_at,
             quarantine_reason
         ) VALUES
             ('41', 'quarantined', 1, now() - interval '1 day', now() - interval '1 hour',
              'new_upload_during_pending'),
             ('42', 'quarantined', 1, now() - interval '1 day', now() - interval '1 hour',
              'result_timeout');",
    )
    .execute(&pool)
    .await?;

    reaper(&pool).reap_transcriptions().await?;

    assert_eq!(
        dispatched_generations(&pool).await?,
        vec![("41".to_owned(), 2)]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_rejection_under_an_older_build_is_reopened_once_per_newest_version(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_stored_track(&pool, "41", 1).await?;
    seed_lyrics(
        &pool,
        "41",
        "plain lyrics the rolled back build keeps rejecting",
    )
    .await?;
    seed_wire(&pool, "41", "rejected", 1, 7).await?;
    sqlx::raw_sql(
        "UPDATE transcription_wire_state SET sync_version = 'old' WHERE sc_track_id = '41';
         INSERT INTO transcription_sync_versions (sync_version, first_seen_at) VALUES
             ('old', now() - interval '10 days'),
             ('new', now() - interval '1 day');",
    )
    .execute(&pool)
    .await?;
    let rejected_again = "UPDATE transcription_wire_state
         SET status = 'rejected', sync_version = 'old', reason = 'low_confidence',
             completed_at = now() - interval '7 hours'
         WHERE sc_track_id = '41'";

    reaper(&pool).reap_transcriptions().await?;
    let first = wire_state(&pool, "41").await?;
    sqlx::query(rejected_again).execute(&pool).await?;
    sqlx::query("DELETE FROM background_jobs")
        .execute(&pool)
        .await?;
    reaper(&pool).reap_transcriptions().await?;
    let second = wire_state(&pool, "41").await?;
    sqlx::query("INSERT INTO transcription_sync_versions (sync_version) VALUES ('newer')")
        .execute(&pool)
        .await?;
    reaper(&pool).reap_transcriptions().await?;
    let third = wire_state(&pool, "41").await?;

    assert_eq!((first.0.as_str(), first.1), ("pending", 3));
    assert_eq!((second.0.as_str(), second.1), ("rejected", 3));
    assert_eq!((third.0.as_str(), third.1), ("pending", 4));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn changed_lyrics_after_a_finished_embedding_are_embedded_again(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    for id in ["41", "42"] {
        seed_stored_track(&pool, id, 1).await?;
        seed_lyrics(&pool, id, "lyrics long enough to need a fresh embedding").await?;
    }
    sqlx::raw_sql(
        "INSERT INTO lyrics_embedding_wire_state (
             sc_track_id, status, first_publish_attempt_at, completed_at
         ) VALUES
             ('41', 'done', now() - interval '1 day', now() - interval '1 day'),
             ('42', 'pending', now() - interval '1 hour', NULL);
         UPDATE lyrics_cache SET embedding_state = 'dispatched' WHERE sc_track_id = '42';",
    )
    .execute(&pool)
    .await?;

    reaper(&pool).reap_embeddings().await?;

    let queued: Vec<String> = sqlx::query_scalar(
        "SELECT dedup_key FROM background_jobs WHERE kind = 'lyrics.embed' ORDER BY dedup_key",
    )
    .fetch_all(&pool)
    .await?;
    let state: Option<String> =
        sqlx::query_scalar("SELECT embedding_state FROM lyrics_cache WHERE sc_track_id = '41'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(queued, vec!["41".to_owned()]);
    assert_eq!(state.as_deref(), Some("queued"));
    Ok(())
}
