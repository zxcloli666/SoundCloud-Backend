use chrono::{TimeZone, Utc};

use super::*;

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE tracks (
             sc_track_id text PRIMARY KEY,
             duration_ms integer NOT NULL,
             needs_duration_resolve boolean NOT NULL DEFAULT false,
             storage_state varchar(16) NOT NULL DEFAULT 'pending',
             storage_quality varchar(4),
             storage_attempts smallint NOT NULL DEFAULT 0,
             s3_verified_at timestamptz,
             s3_missing_at timestamptz,
             hq_upgrade_pending boolean NOT NULL DEFAULT false,
             index_state varchar(16) NOT NULL DEFAULT 'pending',
             indexed_at timestamptz,
             transcribe_state varchar(16),
             transcribe_at timestamptz,
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE lyrics_cache (
             sc_track_id text PRIMARY KEY,
             synced_lrc text,
             plain_text text,
             language varchar(8)
         );
         CREATE TABLE pipeline_event_receipts (
             consumer varchar(96) NOT NULL,
             stream varchar(128) NOT NULL,
             stream_sequence bigint NOT NULL,
             event_published_at timestamptz NOT NULL,
             processed_at timestamptz NOT NULL DEFAULT now(),
             PRIMARY KEY (consumer, stream, stream_sequence, event_published_at)
         );
         CREATE TABLE storage_event_state (
             sc_track_id text PRIMARY KEY REFERENCES tracks(sc_track_id) ON DELETE CASCADE,
             stream varchar(128) NOT NULL,
             stream_sequence bigint NOT NULL,
             event_published_at timestamptz NOT NULL,
             uploaded_generation bigint NOT NULL DEFAULT 0,
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
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE background_job_enqueues (
             id uuid PRIMARY KEY,
             accepted_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE background_jobs (
             id uuid PRIMARY KEY,
             kind varchar(96) NOT NULL,
             lane varchar(16) NOT NULL,
             dedup_key text,
             payload jsonb NOT NULL,
             priority smallint NOT NULL,
             generation bigint NOT NULL DEFAULT 1,
             attempts integer NOT NULL DEFAULT 0,
             max_attempts smallint NOT NULL,
             available_at timestamptz NOT NULL,
             lease_id uuid,
             lease_generation bigint,
             leased_by text,
             lease_expires_at timestamptz,
             last_error text,
             created_at timestamptz NOT NULL DEFAULT now(),
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE UNIQUE INDEX background_jobs_dedup_idx
             ON background_jobs (kind, dedup_key)
             WHERE dedup_key IS NOT NULL;",
    )
    .execute(pool)
    .await?;
    super::super::test_schema::install_audio_index_wire_state(pool).await
}

async fn claim_audio_dispatch(pool: &PgPool, generation: i64) -> anyhow::Result<Option<i32>> {
    Ok(sqlx::query_file_scalar!(
        "queries/indexing/storage/dispatch_audio.sql",
        "42",
        generation
    )
    .fetch_one(pool)
    .await?)
}

async fn insert_pending_transcription(pool: &PgPool, generation: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO transcription_wire_state (
             sc_track_id, status, upload_generation, dispatched_at
         ) VALUES ('42', 'pending', $1, now())",
    )
    .bind(generation)
    .execute(pool)
    .await?;
    Ok(())
}

fn first_dispatch() -> AudioDispatch {
    AudioDispatch {
        sc_track_id: "42".to_owned(),
        upload_generation: 1,
        attempt: 1,
    }
}

async fn insert_track(pool: &PgPool, duration_ms: i32) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, duration_ms)
         VALUES ('42', $1)",
    )
    .bind(duration_ms)
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_plain_lyrics(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, plain_text, language)
         VALUES ('42', repeat('line ', 500), 'en')",
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn queue(pool: &PgPool) -> JobRepository {
    JobRepository::new(pool.clone(), "test".to_owned())
}

fn upload(quality: Option<&str>) -> UploadedAudio {
    UploadedAudio {
        sc_track_id: "42".to_owned(),
        quality: quality.map(|quality| match quality {
            "sq" => "sq",
            "hq" => "hq",
            _ => unreachable!(),
        }),
    }
}

async fn reject(pool: &PgPool, sequence: i64, seconds: i64) -> anyhow::Result<()> {
    sqlx::query_file!(
        "queries/indexing/storage/reject.sql",
        "backend-storage-rejected",
        "STORAGE_EVENTS",
        sequence,
        Utc.timestamp_opt(seconds, 0).unwrap(),
        "42",
        3_i32
    )
    .fetch_one(pool)
    .await?;
    Ok(())
}

fn delivery(sequence: u64, seconds: i64) -> DeliveryContext {
    DeliveryContext {
        consumer: "backend-storage-uploaded".to_owned(),
        stream: "STORAGE_EVENTS".to_owned(),
        stream_sequence: sequence,
        delivery_attempt: 1,
        published_at: Utc.timestamp_opt(seconds, 0).unwrap(),
    }
}

#[test]
fn upload_validation_accepts_real_and_synthetic_events() -> anyhow::Result<()> {
    for quality in [None, Some("sq".to_owned()), Some("hq".to_owned())] {
        validate_upload(StorageTrackUploaded {
            sc_track_id: "soundcloud:tracks:42".to_owned(),
            storage_url: "https://storage.example/redirect/42.m4a".to_owned(),
            quality,
        })?;
    }
    Ok(())
}

#[test]
fn upload_validation_rejects_unsafe_values() {
    let invalid = [
        StorageTrackUploaded {
            sc_track_id: "042".to_owned(),
            storage_url: "https://storage.example/redirect/42.m4a".to_owned(),
            quality: None,
        },
        StorageTrackUploaded {
            sc_track_id: "42".to_owned(),
            storage_url: "file:///tmp/42.m4a".to_owned(),
            quality: None,
        },
        StorageTrackUploaded {
            sc_track_id: "42".to_owned(),
            storage_url: "https://user:secret@storage.example/42.m4a".to_owned(),
            quality: Some("lossless".to_owned()),
        },
    ];

    assert!(
        invalid
            .into_iter()
            .all(|payload| validate_upload(payload).is_err())
    );
}

#[sqlx::test(migrations = false)]
async fn upload_and_redelivery_enqueue_each_dispatch_once(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    insert_track(&pool, 180_000).await?;
    insert_plain_lyrics(&pool).await?;

    let first = apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("sq")),
        &delivery(1, 10),
        420_000,
    )
    .await?;
    let repeated = apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("sq")),
        &delivery(1, 10),
        420_000,
    )
    .await?;

    let state = sqlx::query_as::<_, (String, Option<String>, bool, i64)>(
        "SELECT track.storage_state,
                track.storage_quality,
                track.hq_upgrade_pending,
                event.uploaded_generation
         FROM tracks AS track
         JOIN storage_event_state AS event USING (sc_track_id)
         WHERE track.sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let jobs: Vec<String> = sqlx::query_scalar("SELECT kind FROM background_jobs ORDER BY kind")
        .fetch_all(&pool)
        .await?;
    let payloads: Vec<serde_json::Value> =
        sqlx::query_scalar("SELECT payload FROM background_jobs ORDER BY kind")
            .fetch_all(&pool)
            .await?;
    assert!(first.applied);
    assert!(!repeated.applied);
    assert_eq!(state, ("ok".to_owned(), Some("sq".to_owned()), true, 1));
    assert_eq!(
        jobs,
        vec![
            "indexing.dispatch_audio".to_owned(),
            "lyrics.dispatch_transcription".to_owned()
        ]
    );
    assert!(
        payloads
            .iter()
            .all(|payload| payload["payload"].get("storage_url").is_none())
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn newer_upload_supersedes_jobs_and_stale_upload_is_ignored(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    insert_track(&pool, 180_000).await?;
    insert_plain_lyrics(&pool).await?;

    apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("sq")),
        &delivery(2, 20),
        420_000,
    )
    .await?;
    let stale = apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("hq")),
        &delivery(1, 10),
        420_000,
    )
    .await?;
    let latest = apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("hq")),
        &delivery(3, 30),
        420_000,
    )
    .await?;

    let state = sqlx::query_as::<_, (Option<String>, i64)>(
        "SELECT track.storage_quality, event.uploaded_generation
         FROM tracks AS track
         JOIN storage_event_state AS event USING (sc_track_id)
         WHERE track.sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let generations: Vec<i64> = sqlx::query_scalar(
        "SELECT (payload #>> '{payload,uploaded_generation}')::bigint
         FROM background_jobs ORDER BY kind",
    )
    .fetch_all(&pool)
    .await?;
    assert!(!stale.applied);
    assert_eq!(latest.generation, 2);
    assert_eq!(state, (Some("hq".to_owned()), 2));
    assert_eq!(generations, vec![2, 2]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn upload_without_plain_lyrics_never_dispatches_full_transcription(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    insert_track(&pool, 180_000).await?;

    apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("sq")),
        &delivery(1, 10),
        420_000,
    )
    .await?;

    let transcription_jobs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM background_jobs WHERE kind = 'lyrics.dispatch_transcription'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(transcription_jobs, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn oversized_upload_becomes_terminal_without_dispatch(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    insert_track(&pool, 500_000).await?;

    let applied = apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("sq")),
        &delivery(1, 10),
        420_000,
    )
    .await?;

    let state = sqlx::query_as::<_, (String, String, Option<String>, bool)>(
        "SELECT storage_state, index_state, transcribe_state, hq_upgrade_pending
         FROM tracks WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    assert!(applied.too_long);
    assert_eq!(
        state,
        (
            "too_long".to_owned(),
            "too_long".to_owned(),
            Some("disabled".to_owned()),
            false
        )
    );
    assert_eq!(jobs, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn unknown_track_keeps_the_event_retryable(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;

    let error = apply_upload(
        &pool,
        &queue(&pool),
        &upload(None),
        &delivery(1, 10),
        420_000,
    )
    .await
    .expect_err("unknown track must retry");
    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM pipeline_event_receipts")
        .fetch_one(&pool)
        .await?;
    assert!(error.is_retryable());
    assert_eq!(receipts, 0);
    Ok(())
}

#[test]
fn an_audio_index_task_names_its_generation_and_attempt() -> anyhow::Result<()> {
    let storage_url = Url::parse("https://storage.example/")?;
    let dispatch = AudioDispatch {
        sc_track_id: "42".to_owned(),
        upload_generation: 3,
        attempt: 2,
    };

    let request = audio_index_request(&storage_url, &dispatch)?;

    assert_eq!(audio_index_message_id(&dispatch), "storage-audio:42:3:2");
    assert_eq!(
        serde_json::to_value(&request)?,
        serde_json::json!({
            "sc_track_id": "42",
            "s3_url": "https://storage.example/redirect/soundcloud_tracks_42.m4a",
            "upload_generation": 3,
            "attempt": 2,
        })
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn newer_upload_quarantines_an_active_transcription(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    insert_track(&pool, 180_000).await?;
    insert_plain_lyrics(&pool).await?;
    apply_upload(
        &pool,
        &queue(&pool),
        &upload(None),
        &delivery(1, 10),
        420_000,
    )
    .await?;
    insert_pending_transcription(&pool, 1).await?;

    apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("hq")),
        &delivery(2, 20),
        420_000,
    )
    .await?;

    let state = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT track.transcribe_state,
                wire.status,
                wire.quarantine_reason
         FROM tracks AS track
         JOIN transcription_wire_state AS wire USING (sc_track_id)
         WHERE track.sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let transcription_jobs: i64 = sqlx::query_scalar(
        "SELECT count(*)
         FROM background_jobs
         WHERE kind = 'lyrics.dispatch_transcription'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        state,
        (
            "quarantined".to_owned(),
            "quarantined".to_owned(),
            Some("new_upload_during_pending".to_owned())
        )
    );
    assert_eq!(transcription_jobs, 1);
    Ok(())
}

async fn wait_until_blocked(pool: &PgPool) -> anyhow::Result<()> {
    for _ in 0..500 {
        let blocked: i64 = sqlx::query_scalar(
            "SELECT count(*)
             FROM pg_stat_activity
             WHERE wait_event_type = 'Lock'
               AND state = 'active'
               AND datname = current_database()",
        )
        .fetch_one(pool)
        .await?;
        if blocked > 0 {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    anyhow::bail!("the concurrent upload never blocked on the track lock")
}

#[sqlx::test(migrations = false)]
async fn a_dispatch_committed_during_the_upload_statement_is_still_quarantined(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    insert_track(&pool, 180_000).await?;
    apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("sq")),
        &delivery(1, 10),
        420_000,
    )
    .await?;
    sqlx::query(
        "INSERT INTO audio_index_wire_state (
             sc_track_id, status, upload_generation, dispatched_at, completed_at
         ) VALUES ('42', 'done', 1, now(), now())",
    )
    .execute(&pool)
    .await?;
    apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("sq")),
        &delivery(2, 20),
        420_000,
    )
    .await?;

    let mut dispatcher = pool.begin().await?;
    let attempt =
        sqlx::query_file_scalar!("queries/indexing/storage/dispatch_audio.sql", "42", 2_i64)
            .fetch_one(&mut *dispatcher)
            .await?;
    assert_eq!(attempt, Some(1));

    let concurrent_pool = pool.clone();
    let uploader = tokio::spawn(async move {
        apply_upload(
            &concurrent_pool,
            &queue(&concurrent_pool),
            &upload(Some("hq")),
            &delivery(3, 30),
            420_000,
        )
        .await
    });
    wait_until_blocked(&pool).await?;
    dispatcher.commit().await?;
    let applied = uploader.await??;

    let wire = sqlx::query_as::<_, (String, Option<i64>, Option<String>)>(
        "SELECT status, upload_generation, quarantine_reason
         FROM audio_index_wire_state
         WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(applied.generation, 3);
    assert_eq!(
        wire,
        (
            "quarantined".to_owned(),
            Some(2),
            Some("new_upload_during_pending".to_owned())
        )
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_quality_upgrade_reopens_the_index_of_an_already_indexed_track(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    insert_track(&pool, 180_000).await?;
    apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("sq")),
        &delivery(1, 10),
        420_000,
    )
    .await?;
    sqlx::query(
        "UPDATE tracks SET index_state = 'indexed', indexed_at = now() WHERE sc_track_id = '42'",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO audio_index_wire_state (
             sc_track_id, status, upload_generation, dispatched_at, completed_at
         ) VALUES ('42', 'done', 1, now(), now())",
    )
    .execute(&pool)
    .await?;
    sqlx::query("DELETE FROM background_jobs")
        .execute(&pool)
        .await?;

    let upgraded = apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("hq")),
        &delivery(2, 20),
        420_000,
    )
    .await?;

    let state = sqlx::query_as::<_, (String, bool, i64)>(
        "SELECT track.index_state,
                track.indexed_at IS NOT NULL,
                event.uploaded_generation
         FROM tracks AS track
         JOIN storage_event_state AS event USING (sc_track_id)
         WHERE track.sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let dispatch_generations: Vec<i64> = sqlx::query_scalar(
        "SELECT (payload #>> '{payload,uploaded_generation}')::bigint
         FROM background_jobs
         WHERE kind = 'indexing.dispatch_audio'",
    )
    .fetch_all(&pool)
    .await?;
    let attempt = claim_audio_dispatch(&pool, 2).await?;
    assert_eq!(upgraded.generation, 2);
    assert_eq!(state, ("pending".to_owned(), false, 2));
    assert_eq!(dispatch_generations, vec![2]);
    assert_eq!(attempt, Some(1));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unpublished_dispatch_stays_reopenable_and_is_republished_with_the_same_attempt(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    insert_track(&pool, 180_000).await?;
    apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("sq")),
        &delivery(1, 10),
        420_000,
    )
    .await?;
    let attempt = claim_audio_dispatch(&pool, 1).await?;

    abandon_audio_dispatch(
        &pool,
        &AudioDispatch {
            attempt: 2,
            ..first_dispatch()
        },
    )
    .await;
    let foreign_attempt_status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM audio_index_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    abandon_audio_dispatch(&pool, &first_dispatch()).await;

    let abandoned = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(
        "SELECT status, outcome_reason, quarantine_reason
         FROM audio_index_wire_state
         WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let republished = claim_audio_dispatch(&pool, 1).await?;
    assert_eq!(attempt, Some(1));
    assert_eq!(foreign_attempt_status, "pending");
    assert_eq!(
        abandoned,
        (
            "reopenable".to_owned(),
            Some("dispatch_publish_failed".to_owned()),
            None
        )
    );
    assert_eq!(republished, Some(1));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unpublished_dispatch_never_cuts_a_live_result_lease(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    insert_track(&pool, 180_000).await?;
    apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("sq")),
        &delivery(1, 10),
        420_000,
    )
    .await?;
    claim_audio_dispatch(&pool, 1).await?;
    sqlx::query(
        "UPDATE audio_index_wire_state
         SET result_lease_id = gen_random_uuid(),
             result_lease_expires_at = now() + interval '2 minutes'
         WHERE sc_track_id = '42'",
    )
    .execute(&pool)
    .await?;

    abandon_audio_dispatch(&pool, &first_dispatch()).await;

    let status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM audio_index_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(status, "pending");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_later_rejection_does_not_swallow_an_accepted_upload(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    insert_track(&pool, 180_000).await?;
    apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("sq")),
        &delivery(1, 10),
        420_000,
    )
    .await?;
    sqlx::query(
        "UPDATE tracks SET index_state = 'indexed', indexed_at = now() WHERE sc_track_id = '42'",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO audio_index_wire_state (
             sc_track_id, status, upload_generation, dispatched_at, completed_at
         ) VALUES ('42', 'done', 1, now(), now())",
    )
    .execute(&pool)
    .await?;
    sqlx::query("DELETE FROM background_jobs")
        .execute(&pool)
        .await?;

    reject(&pool, 3, 30).await?;
    let upgraded = apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("hq")),
        &delivery(2, 20),
        420_000,
    )
    .await?;

    let state = sqlx::query_as::<_, (String, i64)>(
        "SELECT track.index_state, event.uploaded_generation
         FROM tracks AS track
         JOIN storage_event_state AS event USING (sc_track_id)
         WHERE track.sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let wire = sqlx::query_as::<_, (String, Option<i64>)>(
        "SELECT status, upload_generation FROM audio_index_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let dispatch_generations: Vec<i64> = sqlx::query_scalar(
        "SELECT (payload #>> '{payload,uploaded_generation}')::bigint
         FROM background_jobs
         WHERE kind = 'indexing.dispatch_audio'",
    )
    .fetch_all(&pool)
    .await?;

    assert!(upgraded.applied);
    assert_eq!(upgraded.generation, 2);
    assert_eq!(state, ("pending".to_owned(), 2));
    assert_eq!(wire, ("done".to_owned(), Some(1)));
    assert_eq!(dispatch_generations, vec![2]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn newer_upload_quarantines_an_active_audio_index(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    insert_track(&pool, 180_000).await?;
    apply_upload(
        &pool,
        &queue(&pool),
        &upload(None),
        &delivery(1, 10),
        420_000,
    )
    .await?;
    let claimed = claim_audio_dispatch(&pool, 1).await?;
    assert_eq!(claimed, Some(1));
    let lease_id = uuid::Uuid::now_v7();
    sqlx::query(
        "UPDATE audio_index_wire_state
         SET result_lease_id = $1,
             result_lease_expires_at = now() + interval '120 seconds'
         WHERE sc_track_id = '42'",
    )
    .bind(lease_id)
    .execute(&pool)
    .await?;

    apply_upload(
        &pool,
        &queue(&pool),
        &upload(Some("hq")),
        &delivery(2, 20),
        420_000,
    )
    .await?;

    let wire = sqlx::query_as::<_, (String, Option<i64>, Option<String>)>(
        "SELECT status, upload_generation, quarantine_reason
         FROM audio_index_wire_state
         WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let surviving_lease = sqlx::query_scalar::<_, Option<uuid::Uuid>>(
        "SELECT result_lease_id FROM audio_index_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(surviving_lease, Some(lease_id));
    let stale_dispatch = claim_audio_dispatch(&pool, 1).await?;
    let current_dispatch = claim_audio_dispatch(&pool, 2).await?;
    assert_eq!(
        wire,
        (
            "quarantined".to_owned(),
            Some(1),
            Some("new_upload_during_pending".to_owned())
        )
    );
    assert_eq!(stale_dispatch, None);
    assert_eq!(current_dispatch, Some(1));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn reinserted_track_advances_past_the_persistent_transcription_epoch(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    insert_track(&pool, 180_000).await?;
    insert_plain_lyrics(&pool).await?;
    apply_upload(
        &pool,
        &queue(&pool),
        &upload(None),
        &delivery(1, 10),
        420_000,
    )
    .await?;
    insert_pending_transcription(&pool, 1).await?;

    sqlx::query("DELETE FROM tracks WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;
    insert_track(&pool, 180_000).await?;
    let uploaded = apply_upload(
        &pool,
        &queue(&pool),
        &upload(None),
        &delivery(2, 20),
        420_000,
    )
    .await?;

    let state = sqlx::query_as::<_, (i64, String, String, Option<String>)>(
        "SELECT storage.uploaded_generation,
                track.transcribe_state,
                wire.status,
                wire.quarantine_reason
         FROM tracks AS track
         JOIN storage_event_state AS storage USING (sc_track_id)
         JOIN transcription_wire_state AS wire USING (sc_track_id)
         WHERE track.sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let dispatch_generation: i64 = sqlx::query_scalar(
        "SELECT generation
         FROM background_jobs
         WHERE kind = 'lyrics.dispatch_transcription'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(uploaded.generation, 2);
    assert_eq!(dispatch_generation, 1);
    assert_eq!(
        state,
        (
            2,
            "quarantined".to_owned(),
            "quarantined".to_owned(),
            Some("new_upload_during_pending".to_owned()),
        )
    );
    Ok(())
}
