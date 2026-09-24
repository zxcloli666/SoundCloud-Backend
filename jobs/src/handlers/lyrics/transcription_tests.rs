use std::collections::BTreeMap;

use backend_contracts::pipeline::{Producer, TranscriptionMode, TranscriptionResult};
use backend_contracts::reasons::{WorkerReason, WorkerStatus};
use chrono::{TimeZone, Utc};

use super::*;

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE tracks (
             sc_track_id text PRIMARY KEY,
             storage_state varchar(16) NOT NULL,
             needs_duration_resolve boolean NOT NULL DEFAULT false,
             transcribe_state varchar(16),
             transcribe_at timestamptz,
             language varchar(8),
             language_confidence real,
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
             confidence double precision,
             placed_share double precision,
             aligned_share double precision,
             lines_total integer,
             lines_unplaced integer,
             result_language varchar(8),
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
             synced_source varchar(16),
             language varchar(8),
             language_confidence real,
             embedded_at timestamptz,
             embedding_state varchar(16),
             created_at timestamp NOT NULL DEFAULT now()
         );
         CREATE TABLE pipeline_event_receipts (
             consumer varchar(96) NOT NULL,
             stream varchar(128) NOT NULL,
             stream_sequence bigint NOT NULL,
             event_published_at timestamptz NOT NULL,
             processed_at timestamptz NOT NULL DEFAULT now(),
             PRIMARY KEY (consumer, stream, stream_sequence, event_published_at)
         );
         CREATE TABLE background_jobs (
             id uuid PRIMARY KEY,
             kind varchar(96) NOT NULL
         );",
    )
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0114_lyrics_synced_version.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

async fn sync_provenance(pool: &PgPool) -> anyhow::Result<(Option<String>, Option<String>)> {
    Ok(sqlx::query_as(
        "SELECT synced_source, synced_version FROM lyrics_cache WHERE sc_track_id = '42'",
    )
    .fetch_one(pool)
    .await?)
}

async fn seed_existing_lyrics(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, plain_text, source)
         VALUES ('42', 'A sufficiently long generated lyrics line', 'genius')",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_pending(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO tracks (
             sc_track_id, storage_state, transcribe_state, transcribe_at
         ) VALUES ('42', 'ok', 'pending', to_timestamp(100));
         INSERT INTO storage_event_state (
             sc_track_id, stream, stream_sequence, event_published_at,
             uploaded_generation, transcription_generation
         ) VALUES ('42', 'STORAGE_EVENTS', 1, to_timestamp(90), 1, 1);
         INSERT INTO transcription_wire_state (
             sc_track_id, status, upload_generation, attempt, dispatched_at
         ) VALUES ('42', 'pending', 1, 1, to_timestamp(100));",
    )
    .execute(pool)
    .await?;
    Ok(())
}

fn delivery(sequence: u64) -> DeliveryContext {
    DeliveryContext {
        consumer: "backend-done-transcribe".to_owned(),
        stream: "PIPELINE_DONE".to_owned(),
        stream_sequence: sequence,
        delivery_attempt: 1,
        published_at: Utc.timestamp_opt(200 + sequence as i64, 0).unwrap(),
    }
}

fn outcome(status: WorkerStatus, reason: Option<WorkerReason>) -> TranscriptionResult {
    TranscriptionResult {
        sc_track_id: "42".to_owned(),
        upload_generation: 1,
        attempt: 1,
        mode: TranscriptionMode::Align,
        status,
        reason,
        detail: None,
        producer: Producer {
            worker_id: "gpu-main".to_owned(),
            build: "test".to_owned(),
            models: BTreeMap::new(),
            sync_version: Some("s2.aaaa.bbbb.cccc".to_owned()),
        },
        sync_version: "s2.aaaa.bbbb.cccc".to_owned(),
        confidence: Some(0.4),
        placed_share: Some(0.975),
        aligned_share: Some(0.9),
        lines_total: Some(40),
        lines_unplaced: Some(1),
        language: Some("en".to_owned()),
        synced_lrc: None,
        words: None,
    }
}

fn aligned_result() -> TranscriptionResult {
    let mut result = outcome(WorkerStatus::Ok, None);
    result.synced_lrc = Some("[00:01.00]A sufficiently long generated lyrics line".to_owned());
    result
}

async fn wire(pool: &PgPool) -> anyhow::Result<(String, Option<String>, Option<i16>, i64)> {
    Ok(sqlx::query_as(
        "SELECT status, reason, result_rank, attempt
         FROM transcription_wire_state
         WHERE sc_track_id = '42'",
    )
    .fetch_one(pool)
    .await?)
}

async fn track_state(pool: &PgPool) -> anyhow::Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT transcribe_state FROM tracks WHERE sc_track_id = '42'")
            .fetch_one(pool)
            .await?,
    )
}

#[sqlx::test(migrations = false)]
async fn an_alignment_and_its_redelivery_commit_once(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_pending(&pool).await?;
    seed_existing_lyrics(&pool).await?;
    let handler = TranscriptionResultHandler::new(pool.clone());

    handler.finish(aligned_result(), delivery(1)).await?;
    handler.finish(aligned_result(), delivery(1)).await?;

    let state = sqlx::query_as::<_, (String, String, Option<String>, String, Option<String>)>(
        "SELECT track.transcribe_state, wire.status, lyrics.synced_lrc, lyrics.source,
                wire.sync_version
         FROM tracks AS track
         JOIN transcription_wire_state AS wire USING (sc_track_id)
         JOIN lyrics_cache AS lyrics USING (sc_track_id)
         WHERE track.sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let metrics = sqlx::query_as::<_, (Option<i32>, Option<i32>, Option<f64>, Option<i16>)>(
        "SELECT lines_total, lines_unplaced, placed_share, result_rank
         FROM transcription_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM pipeline_event_receipts")
        .fetch_one(&pool)
        .await?;
    let versions: Vec<String> =
        sqlx::query_scalar("SELECT sync_version FROM transcription_sync_versions")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        state,
        (
            "done".to_owned(),
            "done".to_owned(),
            Some("[00:01.00]A sufficiently long generated lyrics line".to_owned()),
            "genius".to_owned(),
            Some("s2.aaaa.bbbb.cccc".to_owned()),
        )
    );
    assert_eq!(metrics, (Some(40), Some(1), Some(0.975), Some(4)));
    assert_eq!(receipts, 1);
    assert_eq!(versions, vec!["s2.aaaa.bbbb.cccc".to_owned()]);
    assert_eq!(
        sync_provenance(&pool).await?,
        (
            Some("self_gen".to_owned()),
            Some("s2.aaaa.bbbb.cccc".to_owned())
        )
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn empty_result_disables_only_the_current_epoch(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_pending(&pool).await?;
    seed_existing_lyrics(&pool).await?;

    TranscriptionResultHandler::new(pool.clone())
        .finish(
            outcome(WorkerStatus::Empty, Some(WorkerReason::SilentAudio)),
            delivery(1),
        )
        .await?;

    assert_eq!(track_state(&pool).await?.as_deref(), Some("disabled"));
    assert_eq!(
        wire(&pool).await?,
        (
            "empty".to_owned(),
            Some("silent_audio".to_owned()),
            Some(3),
            1
        )
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn rejected_result_keeps_the_lyrics_and_records_what_to_reevaluate(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_pending(&pool).await?;
    seed_existing_lyrics(&pool).await?;

    TranscriptionResultHandler::new(pool.clone())
        .finish(
            outcome(WorkerStatus::Rejected, Some(WorkerReason::LyricsMismatch)),
            delivery(1),
        )
        .await?;

    let rejected = sqlx::query_as::<_, (String, Option<String>, Option<f64>)>(
        "SELECT status, sync_version, confidence
         FROM transcription_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let synced: Option<String> =
        sqlx::query_scalar("SELECT synced_lrc FROM lyrics_cache WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(track_state(&pool).await?.as_deref(), Some("rejected"));
    assert_eq!(
        rejected,
        (
            "rejected".to_owned(),
            Some("s2.aaaa.bbbb.cccc".to_owned()),
            Some(0.4)
        )
    );
    assert_eq!(synced, None);
    assert_eq!(sync_provenance(&pool).await?, (None, None));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_sync_version_never_labels_an_external_sync(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;

    let refused = sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, synced_lrc, source, synced_source, synced_version)
         VALUES ('42', '[00:01.00] line', 'lrclib', 'lrclib', 's2.aaaa.bbbb.cccc')",
    )
    .execute(&pool)
    .await
    .err()
    .map(|error| error.to_string())
    .unwrap_or_default();

    assert!(
        refused.contains("lyrics_cache_synced_version_valid"),
        "{refused}"
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn missing_audio_is_quarantined_with_its_reason(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_pending(&pool).await?;
    seed_existing_lyrics(&pool).await?;

    TranscriptionResultHandler::new(pool.clone())
        .finish(
            outcome(WorkerStatus::Missing, Some(WorkerReason::AudioNotFound)),
            delivery(1),
        )
        .await?;

    let quarantine: Option<String> = sqlx::query_scalar(
        "SELECT quarantine_reason FROM transcription_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(track_state(&pool).await?.as_deref(), Some("quarantined"));
    assert_eq!(wire(&pool).await?.0, "quarantined");
    assert_eq!(quarantine.as_deref(), Some("audio_not_found"));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_late_alignment_outranks_a_reopenable_failure_of_the_same_attempt(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_pending(&pool).await?;
    seed_existing_lyrics(&pool).await?;
    let handler = TranscriptionResultHandler::new(pool.clone());

    handler
        .finish(
            outcome(WorkerStatus::Failed, Some(WorkerReason::EngineRestarted)),
            delivery(1),
        )
        .await?;
    let reopenable = wire(&pool).await?;
    let reopenable_track = track_state(&pool).await?;
    handler.finish(aligned_result(), delivery(2)).await?;

    assert_eq!(
        reopenable,
        (
            "reopenable".to_owned(),
            Some("engine_restarted".to_owned()),
            Some(1),
            1
        )
    );
    assert_eq!(reopenable_track.as_deref(), Some("pending"));
    assert_eq!(wire(&pool).await?, ("done".to_owned(), None, Some(4), 1));
    assert_eq!(track_state(&pool).await?.as_deref(), Some("done"));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_weaker_outcome_never_overwrites_an_alignment(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_pending(&pool).await?;
    seed_existing_lyrics(&pool).await?;
    let handler = TranscriptionResultHandler::new(pool.clone());

    handler.finish(aligned_result(), delivery(1)).await?;
    handler
        .finish(
            outcome(WorkerStatus::Failed, Some(WorkerReason::DeadlineExceeded)),
            delivery(2),
        )
        .await?;
    handler
        .apply_worker_lost(7, &serde_json::to_vec(&worker_lost_request(1))?)
        .await?;

    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM pipeline_event_receipts")
        .fetch_one(&pool)
        .await?;
    assert_eq!(wire(&pool).await?, ("done".to_owned(), None, Some(4), 1));
    assert_eq!(receipts, 2);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_result_of_an_older_attempt_or_generation_is_only_receipted(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_pending(&pool).await?;
    seed_existing_lyrics(&pool).await?;
    sqlx::query("UPDATE transcription_wire_state SET attempt = 2 WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;
    let handler = TranscriptionResultHandler::new(pool.clone());
    let mut older_generation = aligned_result();
    older_generation.upload_generation = 2;
    older_generation.attempt = 2;

    handler.finish(aligned_result(), delivery(1)).await?;
    handler.finish(older_generation, delivery(2)).await?;

    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM pipeline_event_receipts")
        .fetch_one(&pool)
        .await?;
    let synced: Option<String> =
        sqlx::query_scalar("SELECT synced_lrc FROM lyrics_cache WHERE sc_track_id = '42'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(wire(&pool).await?, ("pending".to_owned(), None, None, 2));
    assert_eq!(synced, None);
    assert_eq!(receipts, 2);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn quarantined_result_is_receipted_without_mutation(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_pending(&pool).await?;
    sqlx::raw_sql(
        "UPDATE transcription_wire_state
         SET status = 'quarantined', quarantine_reason = 'new_upload_during_pending'
         WHERE sc_track_id = '42';
         UPDATE storage_event_state SET uploaded_generation = 2 WHERE sc_track_id = '42';
         UPDATE tracks SET transcribe_state = 'quarantined' WHERE sc_track_id = '42';",
    )
    .execute(&pool)
    .await?;

    TranscriptionResultHandler::new(pool.clone())
        .finish(aligned_result(), delivery(1))
        .await?;

    let lyrics: i64 = sqlx::query_scalar("SELECT count(*) FROM lyrics_cache")
        .fetch_one(&pool)
        .await?;
    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM pipeline_event_receipts")
        .fetch_one(&pool)
        .await?;
    assert_eq!(lyrics, 0);
    assert_eq!(receipts, 1);
    assert_eq!(wire(&pool).await?.0, "quarantined");
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn aligned_result_preserves_an_aggregator_winner(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_pending(&pool).await?;
    sqlx::query(
        "INSERT INTO lyrics_cache (sc_track_id, plain_text, synced_lrc, source, synced_source)
         VALUES ('42', 'aggregator lyrics', '[00:02.00]aggregator lyrics', 'lrclib', 'lrclib')",
    )
    .execute(&pool)
    .await?;

    TranscriptionResultHandler::new(pool.clone())
        .finish(aligned_result(), delivery(1))
        .await?;

    let lyrics = sqlx::query_as::<_, (String, Option<String>, String)>(
        "SELECT plain_text, synced_lrc, source FROM lyrics_cache WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        lyrics,
        (
            "aggregator lyrics".to_owned(),
            Some("[00:02.00]aggregator lyrics".to_owned()),
            "lrclib".to_owned()
        )
    );
    assert_eq!(jobs, 0);
    assert_eq!(
        sync_provenance(&pool).await?,
        (Some("lrclib".to_owned()), None)
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn unknown_track_does_not_consume_the_result(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let error = TranscriptionResultHandler::new(pool.clone())
        .finish(aligned_result(), delivery(1))
        .await
        .expect_err("unknown track must retry");

    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM pipeline_event_receipts")
        .fetch_one(&pool)
        .await?;
    assert!(error.is_retryable());
    assert_eq!(receipts, 0);
    Ok(())
}

fn worker_lost_request(attempt: i64) -> TranscriptionRequest {
    TranscriptionRequest {
        sc_track_id: "42".to_owned(),
        upload_generation: 1,
        attempt,
        audio_url: "https://storage.example/redirect/soundcloud_tracks_42.m4a".to_owned(),
        reference_text: "A sufficiently long generated lyrics line".to_owned(),
        reference_lines_total: 1,
        language: None,
        mode: TranscriptionMode::Align,
    }
}

#[sqlx::test(migrations = false)]
async fn a_lost_worker_makes_the_current_attempt_reopenable(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_pending(&pool).await?;
    let handler = TranscriptionResultHandler::new(pool.clone());

    handler
        .apply_worker_lost(9, &serde_json::to_vec(&worker_lost_request(2))?)
        .await?;
    let stale_attempt = wire(&pool).await?;
    handler
        .apply_worker_lost(9, &serde_json::to_vec(&worker_lost_request(1))?)
        .await?;

    assert_eq!(stale_attempt, ("pending".to_owned(), None, None, 1));
    assert_eq!(
        wire(&pool).await?,
        (
            "reopenable".to_owned(),
            Some("worker_lost".to_owned()),
            Some(1),
            1
        )
    );
    assert_eq!(track_state(&pool).await?.as_deref(), Some("pending"));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_worker_lost_payload_that_is_not_a_task_is_rejected(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;

    let error = TranscriptionResultHandler::new(pool.clone())
        .apply_worker_lost(9, b"{\"sc_track_id\":\"42\"}")
        .await
        .expect_err("a truncated task cannot be reopened");

    assert!(!error.is_retryable());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn only_gated_outcomes_publish_their_sync_version(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_pending(&pool).await?;
    seed_existing_lyrics(&pool).await?;
    let handler = TranscriptionResultHandler::new(pool.clone());
    let mut timeout = outcome(WorkerStatus::Failed, Some(WorkerReason::PublicNodeTimeout));
    timeout.sync_version = "emu-timeout".to_owned();
    let mut rejected = outcome(WorkerStatus::Rejected, Some(WorkerReason::LowConfidence));
    rejected.attempt = 2;

    handler.finish(timeout, delivery(1)).await?;
    let after_timeout: Vec<String> =
        sqlx::query_scalar("SELECT sync_version FROM transcription_sync_versions")
            .fetch_all(&pool)
            .await?;
    sqlx::query("UPDATE transcription_wire_state SET status = 'pending', attempt = 2")
        .execute(&pool)
        .await?;
    handler.finish(rejected, delivery(2)).await?;
    let after_rejection: Vec<String> =
        sqlx::query_scalar("SELECT sync_version FROM transcription_sync_versions")
            .fetch_all(&pool)
            .await?;

    assert!(after_timeout.is_empty());
    assert_eq!(after_rejection, vec!["s2.aaaa.bbbb.cccc".to_owned()]);
    Ok(())
}

async fn sync_versions(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(
        sqlx::query_scalar("SELECT sync_version FROM transcription_sync_versions ORDER BY 1")
            .fetch_all(pool)
            .await?,
    )
}

async fn receipts(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM pipeline_event_receipts")
            .fetch_one(pool)
            .await?,
    )
}

#[sqlx::test(migrations = false)]
async fn only_an_applied_result_of_a_released_build_publishes_its_sync_version(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_pending(&pool).await?;
    seed_existing_lyrics(&pool).await?;
    sqlx::query("UPDATE transcription_wire_state SET attempt = 2")
        .execute(&pool)
        .await?;
    let handler = TranscriptionResultHandler::new(pool.clone());
    let mut foreign_attempt = outcome(WorkerStatus::Rejected, Some(WorkerReason::LowConfidence));
    foreign_attempt.sync_version = "s9.aaaa.bbbb.dddd".to_owned();
    let mut unreleased = outcome(WorkerStatus::Rejected, Some(WorkerReason::LowConfidence));
    unreleased.attempt = 2;
    unreleased.sync_version = "s1.old".to_owned();

    handler.finish(foreign_attempt, delivery(1)).await?;
    let after_foreign_attempt = sync_versions(&pool).await?;
    handler.finish(unreleased, delivery(2)).await?;

    assert!(after_foreign_attempt.is_empty());
    assert_eq!(wire(&pool).await?.0, "rejected");
    assert!(sync_versions(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_second_outcome_of_equal_rank_leaves_the_first_in_place(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_pending(&pool).await?;
    seed_existing_lyrics(&pool).await?;
    let handler = TranscriptionResultHandler::new(pool.clone());

    handler
        .finish(
            outcome(WorkerStatus::Rejected, Some(WorkerReason::LowConfidence)),
            delivery(1),
        )
        .await?;
    handler
        .finish(
            outcome(WorkerStatus::Failed, Some(WorkerReason::UndecodableAudio)),
            delivery(2),
        )
        .await?;
    handler
        .finish(
            outcome(WorkerStatus::Empty, Some(WorkerReason::SilentAudio)),
            delivery(3),
        )
        .await?;

    assert_eq!(
        wire(&pool).await?,
        (
            "rejected".to_owned(),
            Some("low_confidence".to_owned()),
            Some(3),
            1
        )
    );
    assert_eq!(track_state(&pool).await?.as_deref(), Some("rejected"));
    assert_eq!(receipts(&pool).await?, 3);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_second_alignment_of_the_same_attempt_keeps_the_first_lyrics_and_metrics(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_pending(&pool).await?;
    seed_existing_lyrics(&pool).await?;
    let handler = TranscriptionResultHandler::new(pool.clone());
    let mut second = aligned_result();
    second.synced_lrc = Some("[00:05.00]A different alignment".to_owned());
    second.lines_unplaced = Some(9);
    second.sync_version = "s3.aaaa.bbbb.eeee".to_owned();

    handler.finish(aligned_result(), delivery(1)).await?;
    handler.finish(second, delivery(2)).await?;

    let state = sqlx::query_as::<_, (Option<String>, Option<i32>, Option<String>)>(
        "SELECT lyrics.synced_lrc, wire.lines_unplaced, wire.sync_version
         FROM transcription_wire_state AS wire
         JOIN lyrics_cache AS lyrics USING (sc_track_id)
         WHERE wire.sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        state,
        (
            Some("[00:01.00]A sufficiently long generated lyrics line".to_owned()),
            Some(1),
            Some("s2.aaaa.bbbb.cccc".to_owned())
        )
    );
    assert_eq!(
        sync_versions(&pool).await?,
        vec!["s2.aaaa.bbbb.cccc".to_owned()]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_result_for_a_track_that_is_briefly_not_servable_is_retried_not_consumed(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_pending(&pool).await?;
    seed_existing_lyrics(&pool).await?;
    let handler = TranscriptionResultHandler::new(pool.clone());
    let mut unready_errors = Vec::new();

    for unready in [
        "UPDATE tracks SET needs_duration_resolve = true",
        "UPDATE tracks SET storage_state = 'pending'",
    ] {
        sqlx::query(unready).execute(&pool).await?;
        let error = handler
            .finish(aligned_result(), delivery(1))
            .await
            .expect_err("an unservable track must defer its result");
        unready_errors.push(error.is_retryable());
        sqlx::query("UPDATE tracks SET needs_duration_resolve = false, storage_state = 'ok'")
            .execute(&pool)
            .await?;
    }
    let deferred_wire = wire(&pool).await?;
    let deferred_receipts = receipts(&pool).await?;
    handler.finish(aligned_result(), delivery(1)).await?;

    assert_eq!(unready_errors, vec![true, true]);
    assert_eq!(deferred_wire, ("pending".to_owned(), None, None, 1));
    assert_eq!(deferred_receipts, 0);
    assert_eq!(wire(&pool).await?, ("done".to_owned(), None, Some(4), 1));
    assert_eq!(track_state(&pool).await?.as_deref(), Some("done"));
    assert_eq!(receipts(&pool).await?, 1);
    Ok(())
}
