use chrono::{DateTime, TimeZone, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::*;

struct ClaimOutcome {
    known: bool,
    settled: bool,
    claimed: bool,
    demoted: bool,
    deferred: bool,
    busy: bool,
}

struct CommitOutcome {
    already_settled: bool,
    committed: bool,
    indexed: bool,
    invalidated: bool,
    demoted: bool,
    released: bool,
    accepted: bool,
}

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE tracks (
             id uuid PRIMARY KEY,
             sc_track_id text NOT NULL UNIQUE,
             storage_state varchar(16) NOT NULL,
             index_state varchar(16) NOT NULL,
             needs_duration_resolve boolean NOT NULL DEFAULT false,
             indexed_at timestamptz,
             index_attempts smallint NOT NULL DEFAULT 0,
             audio_fingerprint text,
             canonical_track_id uuid,
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE storage_event_state (
             sc_track_id text PRIMARY KEY,
             stream varchar(128) NOT NULL,
             stream_sequence bigint NOT NULL,
             event_published_at timestamptz NOT NULL,
             uploaded_generation bigint NOT NULL,
             updated_at timestamptz NOT NULL DEFAULT now()
         );
         CREATE TABLE pipeline_event_receipts (
             consumer varchar(96) NOT NULL,
             stream varchar(128) NOT NULL,
             stream_sequence bigint NOT NULL,
             event_published_at timestamptz NOT NULL,
             processed_at timestamptz NOT NULL DEFAULT now(),
             PRIMARY KEY (consumer, stream, stream_sequence, event_published_at)
         );",
    )
    .execute(pool)
    .await?;
    super::super::test_schema::install_audio_index_wire_state(pool).await
}

async fn seed_track(pool: &PgPool, index_state: &str, generation: i64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (
             id, sc_track_id, storage_state, index_state, needs_duration_resolve
         ) VALUES ($1, '42', 'ok', $2, false)",
    )
    .bind(Uuid::now_v7())
    .bind(index_state)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO storage_event_state (
             sc_track_id, stream, stream_sequence, event_published_at, uploaded_generation
         ) VALUES ('42', 'STORAGE_EVENTS', 1, to_timestamp(90), $1)",
    )
    .bind(generation)
    .execute(pool)
    .await?;
    Ok(())
}

fn published(sequence: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(200 + sequence, 0).unwrap()
}

async fn dispatch(pool: &PgPool, generation: i64) -> anyhow::Result<bool> {
    Ok(sqlx::query_file_scalar!(
        "queries/indexing/storage/dispatch_audio.sql",
        "42",
        generation
    )
    .fetch_one(pool)
    .await?
    .is_some())
}

async fn claim(
    pool: &PgPool,
    sequence: i64,
    generation: i64,
    lease_id: Uuid,
) -> anyhow::Result<ClaimOutcome> {
    claim_attempt(pool, sequence, generation, 1, lease_id).await
}

async fn claim_attempt(
    pool: &PgPool,
    sequence: i64,
    generation: i64,
    attempt: i32,
    lease_id: Uuid,
) -> anyhow::Result<ClaimOutcome> {
    Ok(sqlx::query_file_as!(
        ClaimOutcome,
        "queries/indexing/result/claim.sql",
        "backend-done-index-audio",
        "PIPELINE_DONE",
        sequence,
        published(sequence),
        "42",
        generation,
        lease_id,
        RESULT_CLAIM_SECONDS,
        attempt
    )
    .fetch_one(pool)
    .await?)
}

async fn commit(
    pool: &PgPool,
    sequence: i64,
    generation: i64,
    lease_id: Uuid,
) -> anyhow::Result<CommitOutcome> {
    commit_attempt(pool, sequence, generation, 1, lease_id).await
}

async fn commit_attempt(
    pool: &PgPool,
    sequence: i64,
    generation: i64,
    attempt: i32,
    lease_id: Uuid,
) -> anyhow::Result<CommitOutcome> {
    Ok(sqlx::query_file_as!(
        CommitOutcome,
        "queries/indexing/result/commit.sql",
        "backend-done-index-audio",
        "PIPELINE_DONE",
        sequence,
        published(sequence),
        "42",
        generation,
        lease_id,
        attempt
    )
    .fetch_one(pool)
    .await?)
}

async fn wire_outcome(pool: &PgPool) -> anyhow::Result<(String, i32, i16, Option<String>)> {
    Ok(sqlx::query_as::<_, (String, i32, i16, Option<String>)>(
        "SELECT status, attempt, outcome_rank, outcome_reason
             FROM audio_index_wire_state
             WHERE sc_track_id = '42'",
    )
    .fetch_one(pool)
    .await?)
}

async fn reopen(pool: &PgPool, max_attempts: i32) -> anyhow::Result<Vec<(String, i64, i32)>> {
    Ok(sqlx::query_file!(
        "queries/indexing/reopen_dispatches.sql",
        10_i64,
        3_600_i64,
        max_attempts
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| (row.sc_track_id, row.upload_generation, row.attempt))
    .collect())
}

async fn age_wire(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query("UPDATE audio_index_wire_state SET updated_at = now() - interval '2 hours'")
        .execute(pool)
        .await?;
    Ok(())
}

async fn track_state(pool: &PgPool) -> anyhow::Result<(String, bool)> {
    Ok(sqlx::query_as::<_, (String, bool)>(
        "SELECT index_state, indexed_at IS NOT NULL FROM tracks WHERE sc_track_id = '42'",
    )
    .fetch_one(pool)
    .await?)
}

async fn wire_state(pool: &PgPool) -> anyhow::Result<(String, Option<i64>)> {
    Ok(sqlx::query_as::<_, (String, Option<i64>)>(
        "SELECT status, upload_generation FROM audio_index_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(pool)
    .await?)
}

async fn wire_lease(pool: &PgPool) -> anyhow::Result<Option<Uuid>> {
    Ok(sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT result_lease_id FROM audio_index_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(pool)
    .await?)
}

fn unreachable_qdrant() -> anyhow::Result<QdrantProvisioner> {
    QdrantProvisioner::connect(&crate::config::QdrantConfig {
        grpc_url: "http://127.0.0.1:1".to_owned(),
        api_key: String::new().into(),
    })
}

fn delivery(sequence: i64) -> crate::bus::DeliveryContext {
    crate::bus::DeliveryContext {
        consumer: "backend-done-index-audio".to_owned(),
        stream: "PIPELINE_DONE".to_owned(),
        stream_sequence: sequence as u64,
        delivery_attempt: 1,
        published_at: published(sequence),
    }
}

async fn fingerprint(pool: &PgPool, sc_track_id: &str, value: &str) -> anyhow::Result<()> {
    let prefix = value
        .chars()
        .take(FINGERPRINT_PREFIX_CHARS)
        .collect::<String>();
    let mut transaction = pool.begin().await?;
    lock_fingerprint(&mut transaction, &prefix).await?;
    apply_fingerprint(&mut transaction, sc_track_id, value, &prefix).await?;
    transaction.commit().await?;
    Ok(())
}

async fn receipts(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pipeline_event_receipts")
            .fetch_one(pool)
            .await?,
    )
}

fn producer() -> backend_contracts::pipeline::Producer {
    backend_contracts::pipeline::Producer {
        worker_id: "gpu-main".to_owned(),
        build: "test".to_owned(),
        models: std::collections::BTreeMap::new(),
        sync_version: None,
    }
}

fn result() -> AudioIndexResult {
    AudioIndexResult {
        sc_track_id: "42".to_owned(),
        upload_generation: 1,
        attempt: 1,
        status: WorkerStatus::Ok,
        reason: None,
        detail: None,
        producer: producer(),
        mert: Some(vec![0.25; TRACKS_MERT_DIMENSIONS as usize]),
        clap: Some(vec![0.5; TRACKS_CLAP_DIMENSIONS as usize]),
        fingerprint: Some(" fingerprint ".to_owned()),
    }
}

fn settled(status: WorkerStatus, reason: WorkerReason) -> AudioIndexResult {
    AudioIndexResult {
        status,
        reason: Some(reason),
        detail: Some("worker detail".to_owned()),
        mert: None,
        clap: None,
        fingerprint: None,
        ..result()
    }
}

fn lost_task(generation: i64, attempt: i64) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&AudioIndexRequest {
        sc_track_id: "42".to_owned(),
        s3_url: "https://storage.example/redirect/soundcloud_tracks_42.m4a".to_owned(),
        upload_generation: generation,
        attempt,
    })?)
}

fn indexed(result: AudioIndexResult) -> anyhow::Result<AudioIndex> {
    match validate_result(result)? {
        AudioResult::Indexed(index) => Ok(index),
        AudioResult::Settled { .. } => anyhow::bail!("an ok result must carry vectors"),
    }
}

fn outcome(result: AudioIndexResult) -> anyhow::Result<AudioOutcome> {
    match validate_result(result)? {
        AudioResult::Settled { outcome, .. } => Ok(outcome),
        AudioResult::Indexed(_) => anyhow::bail!("a result without vectors must settle"),
    }
}

#[test]
fn accepts_a_complete_audio_index() -> anyhow::Result<()> {
    let index = indexed(result())?;

    assert_eq!(index.point_id, 42);
    assert_eq!(index.upload_generation, 1);
    assert_eq!(index.attempt, 1);
    assert_eq!(index.fingerprint.as_deref(), Some("fingerprint"));
    Ok(())
}

#[test]
fn rejects_non_canonical_ids_and_invalid_vectors() {
    let mut invalid_id = result();
    invalid_id.sc_track_id = "042".to_owned();
    let mut invalid_mert = result();
    if let Some(mert) = invalid_mert.mert.as_mut() {
        mert.pop();
    }
    let mut invalid_clap = result();
    if let Some(first) = invalid_clap.clap.as_mut().and_then(|clap| clap.first_mut()) {
        *first = f32::NAN;
    }
    let mut missing_clap = result();
    missing_clap.clap = None;

    assert!(validate_result(invalid_id).is_err());
    assert!(validate_result(invalid_mert).is_err());
    assert!(validate_result(invalid_clap).is_err());
    assert!(validate_result(missing_clap).is_err());
}

#[test]
fn rejects_an_uncorrelated_generation_or_attempt() {
    let mut missing = result();
    missing.upload_generation = 0;
    let mut negative = result();
    negative.upload_generation = -1;
    let mut no_attempt = result();
    no_attempt.attempt = 0;
    let mut huge_attempt = result();
    huge_attempt.attempt = i64::from(i32::MAX) + 1;

    for invalid in [missing, negative, no_attempt, huge_attempt] {
        assert!(validate_result(invalid).is_err());
    }
}

#[test]
fn rejects_an_oversized_fingerprint() {
    let mut invalid_fingerprint = result();
    invalid_fingerprint.fingerprint = Some("x".repeat(MAX_FINGERPRINT_BYTES + 1));

    assert!(validate_result(invalid_fingerprint).is_err());
}

#[test]
fn a_status_must_carry_a_reason_of_its_own_kind() {
    let mut reason_on_ok = result();
    reason_on_ok.reason = Some(WorkerReason::SilentAudio);
    let mut no_reason = settled(WorkerStatus::Missing, WorkerReason::AudioNotFound);
    no_reason.reason = None;
    let foreign_reason = settled(WorkerStatus::Empty, WorkerReason::AudioNotFound);

    for invalid in [reason_on_ok, no_reason, foreign_reason] {
        assert!(validate_result(invalid).is_err());
    }
}

#[test]
fn outcomes_are_ranked_and_only_lane_reopenable_reasons_reopen() -> anyhow::Result<()> {
    let cases = [
        (
            WorkerStatus::Empty,
            WorkerReason::SilentAudio,
            "terminal",
            3,
        ),
        (
            WorkerStatus::Missing,
            WorkerReason::AudioForbidden,
            "terminal",
            3,
        ),
        (
            WorkerStatus::Failed,
            WorkerReason::UndecodableAudio,
            "terminal",
            3,
        ),
        (
            WorkerStatus::Failed,
            WorkerReason::DownloadFailed,
            "terminal",
            2,
        ),
        (
            WorkerStatus::Failed,
            WorkerReason::EngineRestarted,
            "reopenable",
            1,
        ),
        (
            WorkerStatus::Failed,
            WorkerReason::PublicNodeTimeout,
            "reopenable",
            1,
        ),
    ];
    for (status, reason, wire_status, rank) in cases {
        let outcome = outcome(settled(status, reason))?;
        assert_eq!(outcome.wire_status(), wire_status, "{}", reason.as_str());
        assert_eq!(outcome.rank(), rank, "{}", reason.as_str());
    }
    Ok(())
}

#[test]
fn a_lost_task_becomes_a_reopenable_worker_lost_outcome() -> anyhow::Result<()> {
    let outcome = worker_lost_outcome(&lost_task(3, 2)?)?;

    assert_eq!(outcome.upload_generation, 3);
    assert_eq!(outcome.attempt, 2);
    assert_eq!(outcome.reason, WorkerReason::WorkerLost);
    assert_eq!(outcome.wire_status(), "reopenable");
    assert_eq!(outcome.rank(), 1);
    assert!(worker_lost_outcome(b"not json").is_err());
    assert!(worker_lost_outcome(&lost_task(0, 1)?).is_err());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn dispatch_claims_a_generation_until_it_completes(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;

    let first = dispatch(&pool, 1).await?;
    let retry = dispatch(&pool, 1).await?;
    let stale = dispatch(&pool, 0).await?;
    let lease_id = Uuid::now_v7();
    claim(&pool, 1, 1, lease_id).await?;
    commit(&pool, 1, 1, lease_id).await?;
    let after_commit = dispatch(&pool, 1).await?;

    assert!(first);
    assert!(retry);
    assert!(!stale);
    assert!(!after_commit);
    assert_eq!(wire_state(&pool).await?, ("done".to_owned(), Some(1)));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_completed_generation_is_never_reopened_by_its_own_dispatch(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let lease_id = Uuid::now_v7();
    claim(&pool, 1, 1, lease_id).await?;
    commit(&pool, 1, 1, lease_id).await?;
    sqlx::query("UPDATE tracks SET index_state = 'pending', indexed_at = NULL")
        .execute(&pool)
        .await?;

    let same_generation = dispatch(&pool, 1).await?;
    let wire_after_same_generation = wire_state(&pool).await?;
    sqlx::query("UPDATE storage_event_state SET uploaded_generation = 2")
        .execute(&pool)
        .await?;
    let next_generation = dispatch(&pool, 2).await?;

    assert!(!same_generation);
    assert_eq!(wire_after_same_generation, ("done".to_owned(), Some(1)));
    assert!(next_generation);
    assert_eq!(wire_state(&pool).await?, ("pending".to_owned(), Some(2)));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_correlated_result_commits_once(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let lease_id = Uuid::now_v7();

    let claimed = claim(&pool, 1, 1, lease_id).await?;
    let committed = commit(&pool, 1, 1, lease_id).await?;
    let redelivered = claim(&pool, 1, 1, Uuid::now_v7()).await?;

    assert!(claimed.known && claimed.claimed && !claimed.settled && !claimed.busy);
    assert!(committed.committed && committed.indexed && committed.accepted);
    assert!(!committed.demoted && !committed.released);
    assert!(redelivered.settled && !redelivered.claimed);
    assert_eq!(track_state(&pool).await?, ("indexed".to_owned(), true));
    assert_eq!(wire_state(&pool).await?, ("done".to_owned(), Some(1)));
    assert_eq!(receipts(&pool).await?, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_stale_generation_never_reaches_the_vector_store(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 2).await?;
    dispatch(&pool, 2).await?;

    let stale = claim(&pool, 1, 1, Uuid::now_v7()).await?;

    assert!(stale.known && stale.settled);
    assert!(!stale.claimed && !stale.busy);
    assert_eq!(track_state(&pool).await?, ("pending".to_owned(), false));
    assert_eq!(wire_state(&pool).await?, ("pending".to_owned(), Some(2)));
    assert_eq!(receipts(&pool).await?, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unrequested_result_is_dismissed(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;

    let unrequested = claim(&pool, 1, 1, Uuid::now_v7()).await?;

    assert!(unrequested.known && unrequested.settled && !unrequested.claimed);
    assert_eq!(track_state(&pool).await?, ("pending".to_owned(), false));
    assert_eq!(receipts(&pool).await?, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn unknown_track_does_not_consume_the_result(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;

    let unknown = claim(&pool, 1, 1, Uuid::now_v7()).await?;

    assert!(!unknown.known && !unknown.settled && !unknown.claimed);
    assert_eq!(receipts(&pool).await?, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_second_applier_waits_for_the_live_lease(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;

    let owner = claim(&pool, 1, 1, Uuid::now_v7()).await?;
    let contender = claim(&pool, 2, 1, Uuid::now_v7()).await?;

    assert!(owner.claimed);
    assert!(contender.busy && !contender.claimed && !contender.settled);
    assert_eq!(receipts(&pool).await?, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_redelivery_retakes_its_own_live_lease(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;

    let first = claim(&pool, 1, 1, Uuid::now_v7()).await?;
    let retaken_lease = Uuid::now_v7();
    let redelivery = claim(&pool, 1, 1, retaken_lease).await?;
    let stranger = claim(&pool, 2, 1, Uuid::now_v7()).await?;

    assert!(first.claimed);
    assert!(redelivery.claimed && !redelivery.busy);
    assert!(stranger.busy && !stranger.claimed);
    assert_eq!(wire_lease(&pool).await?, Some(retaken_lease));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_deferred_result_records_that_the_worker_answered(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    sqlx::query("UPDATE tracks SET storage_state = 'pending' WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;

    let deferred = claim(&pool, 1, 1, Uuid::now_v7()).await?;

    let answered = sqlx::query_as::<_, (bool, Option<Uuid>)>(
        "SELECT result_published_at IS NOT NULL, result_lease_id
         FROM audio_index_wire_state
         WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert!(deferred.deferred && !deferred.claimed && !deferred.settled);
    assert_eq!(answered, (true, None));
    assert_eq!(receipts(&pool).await?, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_expired_lease_is_taken_over_and_the_loser_cannot_commit(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let expired_lease = Uuid::now_v7();
    claim(&pool, 1, 1, expired_lease).await?;
    sqlx::query(
        "UPDATE audio_index_wire_state
         SET result_lease_expires_at = now() - interval '1 second'
         WHERE sc_track_id = '42'",
    )
    .execute(&pool)
    .await?;

    let successor_lease = Uuid::now_v7();
    let successor = claim(&pool, 2, 1, successor_lease).await?;
    let loser = commit(&pool, 1, 1, expired_lease).await?;
    let winner = commit(&pool, 2, 1, successor_lease).await?;

    assert!(successor.claimed);
    assert!(!loser.committed && !loser.demoted && !loser.released);
    assert!(winner.committed && winner.indexed);
    assert_eq!(track_state(&pool).await?, ("indexed".to_owned(), true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_superseded_writer_demotes_the_track_it_may_have_overwritten(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "indexed", 2).await?;
    sqlx::query("UPDATE tracks SET indexed_at = now() WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO audio_index_wire_state (
             sc_track_id, status, upload_generation, dispatched_at, completed_at
         ) VALUES ('42', 'done', 2, now(), now())",
    )
    .execute(&pool)
    .await?;

    let superseded = commit(&pool, 1, 1, Uuid::now_v7()).await?;

    assert!(!superseded.committed && superseded.invalidated && superseded.demoted);
    assert!(superseded.accepted);
    assert_eq!(track_state(&pool).await?, ("pending".to_owned(), false));
    assert_eq!(
        wire_state(&pool).await?,
        ("quarantined".to_owned(), Some(2))
    );
    assert_eq!(receipts(&pool).await?, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_result_for_an_unservable_track_is_deferred_not_consumed(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    sqlx::query("UPDATE tracks SET storage_state = 'missing' WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;

    let deferred = claim(&pool, 1, 1, Uuid::now_v7()).await?;
    let receipts_while_deferred = receipts(&pool).await?;
    sqlx::query("UPDATE tracks SET storage_state = 'ok' WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;
    let recovered = claim(&pool, 1, 1, Uuid::now_v7()).await?;

    assert!(deferred.known && deferred.deferred);
    assert!(!deferred.claimed && !deferred.settled && !deferred.busy);
    assert_eq!(receipts_while_deferred, 0);
    assert!(recovered.claimed && !recovered.deferred);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_result_from_a_closed_epoch_invalidates_the_indexed_track(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "indexed", 2).await?;
    sqlx::query("UPDATE tracks SET indexed_at = now() WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO audio_index_wire_state (
             sc_track_id, status, upload_generation, dispatched_at, completed_at
         ) VALUES ('42', 'done', 2, now(), now())",
    )
    .execute(&pool)
    .await?;

    let stale = claim(&pool, 1, 1, Uuid::now_v7()).await?;

    assert!(stale.settled && stale.demoted && !stale.claimed);
    assert_eq!(track_state(&pool).await?, ("pending".to_owned(), false));
    assert_eq!(
        wire_state(&pool).await?,
        ("quarantined".to_owned(), Some(2))
    );
    assert_eq!(receipts(&pool).await?, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_defenced_writer_leaves_its_own_redelivery_usable(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let lost_lease = Uuid::now_v7();
    claim(&pool, 1, 1, lost_lease).await?;
    sqlx::query(
        "UPDATE audio_index_wire_state
         SET result_lease_expires_at = now() - interval '1 second'
         WHERE sc_track_id = '42'",
    )
    .execute(&pool)
    .await?;
    let successor_lease = Uuid::now_v7();
    assert!(claim(&pool, 1, 1, successor_lease).await?.claimed);

    let loser = commit(&pool, 1, 1, lost_lease).await?;
    let receipts_after_loser = receipts(&pool).await?;
    let winner = commit(&pool, 1, 1, successor_lease).await?;

    assert!(!loser.committed && !loser.invalidated && !loser.accepted);
    assert_eq!(receipts_after_loser, 0);
    assert!(winner.committed && winner.indexed && winner.accepted);
    assert_eq!(track_state(&pool).await?, ("indexed".to_owned(), true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_new_upload_cannot_let_two_generations_write_at_once(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let first_lease = Uuid::now_v7();
    assert!(claim(&pool, 1, 1, first_lease).await?.claimed);

    sqlx::query("UPDATE storage_event_state SET uploaded_generation = 2 WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;
    sqlx::query(
        "UPDATE audio_index_wire_state
         SET status = 'quarantined',
             completed_at = now(),
             quarantine_reason = 'new_upload_during_pending'
         WHERE sc_track_id = '42'",
    )
    .execute(&pool)
    .await?;
    assert!(dispatch(&pool, 2).await?);
    let lease_after_dispatch = wire_lease(&pool).await?;

    let successor_blocked = claim(&pool, 2, 2, Uuid::now_v7()).await?;
    let superseded = commit(&pool, 1, 1, first_lease).await?;
    let state_after_supersede = track_state(&pool).await?;
    let second_lease = Uuid::now_v7();
    let successor = claim(&pool, 2, 2, second_lease).await?;
    let committed = commit(&pool, 2, 2, second_lease).await?;

    assert_eq!(lease_after_dispatch, Some(first_lease));
    assert!(successor_blocked.busy && !successor_blocked.claimed && !successor_blocked.settled);
    assert!(!superseded.committed && !superseded.demoted && superseded.released);
    assert_eq!(state_after_supersede, ("pending".to_owned(), false));
    assert!(successor.claimed);
    assert!(committed.committed && committed.indexed);
    assert_eq!(track_state(&pool).await?, ("indexed".to_owned(), true));
    assert_eq!(wire_state(&pool).await?, ("done".to_owned(), Some(2)));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_expired_lease_does_not_survive_a_new_dispatch(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    claim(&pool, 1, 1, Uuid::now_v7()).await?;
    sqlx::query(
        "UPDATE audio_index_wire_state
         SET result_lease_expires_at = now() - interval '1 second'
         WHERE sc_track_id = '42'",
    )
    .execute(&pool)
    .await?;
    sqlx::query("UPDATE storage_event_state SET uploaded_generation = 2 WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;

    assert!(dispatch(&pool, 2).await?);

    let lease = sqlx::query_as::<_, (Option<Uuid>, Option<String>)>(
        "SELECT result_lease_id, result_consumer
         FROM audio_index_wire_state
         WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(lease, (None, None));
    assert!(claim(&pool, 2, 2, Uuid::now_v7()).await?.claimed);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_duplicate_delivery_of_the_current_generation_never_demotes(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let lease_id = Uuid::now_v7();
    claim(&pool, 1, 1, lease_id).await?;
    commit(&pool, 1, 1, lease_id).await?;

    let duplicate = commit(&pool, 2, 1, Uuid::now_v7()).await?;

    assert!(!duplicate.committed && !duplicate.demoted);
    assert_eq!(track_state(&pool).await?, ("indexed".to_owned(), true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_redelivered_commit_is_already_settled(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let lease_id = Uuid::now_v7();
    claim(&pool, 1, 1, lease_id).await?;

    let first = commit(&pool, 1, 1, lease_id).await?;
    let repeated = commit(&pool, 1, 1, lease_id).await?;

    assert!(first.committed);
    assert!(repeated.already_settled && !repeated.committed && !repeated.demoted);
    assert_eq!(receipts(&pool).await?, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_result_that_never_commits_writes_no_fingerprint(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 2).await?;
    dispatch(&pool, 2).await?;
    let handler = AudioIndexResultHandler::new(pool.clone(), unreachable_qdrant()?);

    handler
        .finish(
            AudioIndexResult {
                fingerprint: Some("stale-epoch-fingerprint".to_owned()),
                ..result()
            },
            delivery(1),
        )
        .await?;

    let stored = sqlx::query_scalar::<_, Option<String>>(
        "SELECT audio_fingerprint FROM tracks WHERE sc_track_id = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(stored, None);
    assert_eq!(receipts(&pool).await?, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_fingerprint_never_moves_a_track_between_canonical_groups(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let own_group = Uuid::now_v7();
    let neighbour_group = Uuid::now_v7();
    for (sc_track_id, canonical) in [("41", neighbour_group), ("42", own_group)] {
        sqlx::query(
            "INSERT INTO tracks (
                 id, sc_track_id, storage_state, index_state, needs_duration_resolve,
                 audio_fingerprint, canonical_track_id
             ) VALUES ($1, $2, 'ok', 'indexed', false, 'shared-fingerprint', $3)",
        )
        .bind(Uuid::now_v7())
        .bind(sc_track_id)
        .bind(canonical)
        .execute(&pool)
        .await?;
    }

    fingerprint(&pool, "42", "shared-fingerprint").await?;

    let canonical = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT canonical_track_id FROM tracks ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(canonical, vec![Some(neighbour_group), Some(own_group)]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn concurrent_fingerprints_join_one_canonical_group(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    for sc_track_id in ["41", "42"] {
        sqlx::query(
            "INSERT INTO tracks (
                 id, sc_track_id, storage_state, index_state, needs_duration_resolve
             ) VALUES ($1, $2, 'ok', 'indexed', false)",
        )
        .bind(Uuid::now_v7())
        .bind(sc_track_id)
        .execute(&pool)
        .await?;
    }

    let (first, second) = tokio::join!(
        fingerprint(&pool, "41", "shared-fingerprint"),
        fingerprint(&pool, "42", "shared-fingerprint"),
    );
    first?;
    second?;

    let canonical = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT canonical_track_id FROM tracks ORDER BY sc_track_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(canonical.len(), 2);
    assert!(canonical[0].is_some());
    assert_eq!(canonical[0], canonical[1]);
    Ok(())
}

fn attempt(result: AudioIndexResult, attempt: i64) -> AudioIndexResult {
    AudioIndexResult { attempt, ..result }
}

#[sqlx::test(migrations = false)]
async fn a_terminal_outcome_closes_the_generation(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let handler = AudioIndexResultHandler::new(pool.clone(), unreachable_qdrant()?);

    handler
        .finish(
            settled(WorkerStatus::Failed, WorkerReason::UndecodableAudio),
            delivery(1),
        )
        .await?;
    age_wire(&pool).await?;

    assert_eq!(
        wire_outcome(&pool).await?,
        (
            "terminal".to_owned(),
            1,
            3,
            Some("undecodable_audio".to_owned())
        )
    );
    assert!(reopen(&pool, 8).await?.is_empty());
    assert!(!dispatch(&pool, 1).await?);
    assert_eq!(track_state(&pool).await?, ("failed".to_owned(), false));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn every_terminal_outcome_but_a_forbidden_url_fails_the_track_of_its_generation(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    let handler = AudioIndexResultHandler::new(pool.clone(), unreachable_qdrant()?);
    let mut observed = Vec::new();

    for (status, reason) in [
        (WorkerStatus::Empty, WorkerReason::SilentAudio),
        (WorkerStatus::Missing, WorkerReason::AudioNotFound),
        (WorkerStatus::Failed, WorkerReason::DeadlineExceeded),
        (WorkerStatus::Missing, WorkerReason::AudioForbidden),
    ] {
        sqlx::query("DELETE FROM audio_index_wire_state")
            .execute(&pool)
            .await?;
        sqlx::query("UPDATE tracks SET index_state = 'pending'")
            .execute(&pool)
            .await?;
        dispatch(&pool, 1).await?;
        handler.finish(settled(status, reason), delivery(1)).await?;
        observed.push((reason.as_str(), track_state(&pool).await?.0));
    }

    assert_eq!(
        observed,
        vec![
            ("silent_audio", "failed".to_owned()),
            ("audio_not_found", "failed".to_owned()),
            ("deadline_exceeded", "failed".to_owned()),
            ("audio_forbidden", "pending".to_owned()),
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_terminal_outcome_of_a_replaced_upload_leaves_the_track_pending(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    sqlx::query("UPDATE storage_event_state SET uploaded_generation = 2")
        .execute(&pool)
        .await?;
    let handler = AudioIndexResultHandler::new(pool.clone(), unreachable_qdrant()?);

    handler
        .finish(
            settled(WorkerStatus::Failed, WorkerReason::UndecodableAudio),
            delivery(1),
        )
        .await?;

    assert_eq!(wire_outcome(&pool).await?.0, "terminal");
    assert_eq!(track_state(&pool).await?, ("pending".to_owned(), false));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_lost_task_is_redispatched_with_the_next_attempt_after_the_cooldown(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let handler = AudioIndexResultHandler::new(pool.clone(), unreachable_qdrant()?);

    handler.apply_worker_lost(7, &lost_task(1, 1)?).await?;
    let lost = wire_outcome(&pool).await?;
    let during_cooldown = reopen(&pool, 8).await?;
    age_wire(&pool).await?;
    let reopened = reopen(&pool, 8).await?;
    let reopened_wire = wire_outcome(&pool).await?;
    let late_first_attempt = claim_attempt(&pool, 3, 1, 1, Uuid::now_v7()).await?;
    let lease_id = Uuid::now_v7();
    let second_attempt = claim_attempt(&pool, 4, 1, 2, lease_id).await?;
    let committed = commit_attempt(&pool, 4, 1, 2, lease_id).await?;

    assert_eq!(
        lost,
        (
            "reopenable".to_owned(),
            1,
            1,
            Some("worker_lost".to_owned())
        )
    );
    assert!(during_cooldown.is_empty());
    assert_eq!(reopened, vec![("42".to_owned(), 1, 2)]);
    assert_eq!(reopened_wire, ("pending".to_owned(), 2, 0, None));
    assert!(late_first_attempt.settled && !late_first_attempt.claimed);
    assert!(second_attempt.claimed);
    assert!(committed.committed && committed.indexed);
    assert_eq!(wire_outcome(&pool).await?, ("done".to_owned(), 2, 4, None));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_forbidden_audio_url_is_reissued_until_the_attempts_run_out(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let handler = AudioIndexResultHandler::new(pool.clone(), unreachable_qdrant()?);
    let forbidden = || settled(WorkerStatus::Missing, WorkerReason::AudioForbidden);

    handler.finish(forbidden(), delivery(1)).await?;
    age_wire(&pool).await?;
    let reissued = reopen(&pool, 2).await?;
    handler.finish(attempt(forbidden(), 2), delivery(2)).await?;
    age_wire(&pool).await?;
    let exhausted = reopen(&pool, 2).await?;

    assert_eq!(reissued, vec![("42".to_owned(), 1, 2)]);
    assert!(exhausted.is_empty());
    assert_eq!(
        wire_outcome(&pool).await?,
        (
            "terminal".to_owned(),
            2,
            3,
            Some("audio_forbidden".to_owned())
        )
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_late_ok_outranks_an_earlier_failure_of_the_same_attempt(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let handler = AudioIndexResultHandler::new(pool.clone(), unreachable_qdrant()?);

    handler
        .finish(
            settled(WorkerStatus::Failed, WorkerReason::DeadlineExceeded),
            delivery(1),
        )
        .await?;
    let failed = wire_outcome(&pool).await?;
    let lease_id = Uuid::now_v7();
    let claimed = claim(&pool, 2, 1, lease_id).await?;
    let committed = commit(&pool, 2, 1, lease_id).await?;

    assert_eq!(
        failed,
        (
            "terminal".to_owned(),
            1,
            2,
            Some("deadline_exceeded".to_owned())
        )
    );
    assert!(claimed.claimed);
    assert!(committed.committed && committed.indexed);
    assert_eq!(wire_outcome(&pool).await?, ("done".to_owned(), 1, 4, None));
    assert_eq!(track_state(&pool).await?, ("indexed".to_owned(), true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn no_failure_overwrites_an_applied_ok(pool: PgPool) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let lease_id = Uuid::now_v7();
    claim(&pool, 1, 1, lease_id).await?;
    commit(&pool, 1, 1, lease_id).await?;
    let handler = AudioIndexResultHandler::new(pool.clone(), unreachable_qdrant()?);

    handler
        .finish(
            settled(WorkerStatus::Failed, WorkerReason::DeadlineExceeded),
            delivery(2),
        )
        .await?;
    handler.apply_worker_lost(9, &lost_task(1, 1)?).await?;

    assert_eq!(wire_outcome(&pool).await?, ("done".to_owned(), 1, 4, None));
    assert_eq!(track_state(&pool).await?, ("indexed".to_owned(), true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_terminal_outcome_outranks_worker_lost_in_either_order(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let handler = AudioIndexResultHandler::new(pool.clone(), unreachable_qdrant()?);
    let silent = || settled(WorkerStatus::Empty, WorkerReason::SilentAudio);
    let expected = ("terminal".to_owned(), 1, 3, Some("silent_audio".to_owned()));

    handler.apply_worker_lost(5, &lost_task(1, 1)?).await?;
    handler.finish(silent(), delivery(1)).await?;
    let lost_then_terminal = wire_outcome(&pool).await?;
    handler.apply_worker_lost(5, &lost_task(1, 1)?).await?;
    let terminal_then_lost = wire_outcome(&pool).await?;

    assert_eq!(lost_then_terminal, expected);
    assert_eq!(terminal_then_lost, expected);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_outcome_of_another_generation_or_attempt_changes_nothing(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 2).await?;
    dispatch(&pool, 2).await?;
    let handler = AudioIndexResultHandler::new(pool.clone(), unreachable_qdrant()?);

    handler
        .finish(
            settled(WorkerStatus::Missing, WorkerReason::AudioNotFound),
            delivery(1),
        )
        .await?;
    handler
        .finish(
            AudioIndexResult {
                upload_generation: 2,
                ..attempt(
                    settled(WorkerStatus::Missing, WorkerReason::AudioNotFound),
                    2,
                )
            },
            delivery(2),
        )
        .await?;
    handler.apply_worker_lost(3, &lost_task(1, 1)?).await?;

    assert_eq!(
        wire_outcome(&pool).await?,
        ("pending".to_owned(), 1, 0, None)
    );
    assert_eq!(wire_state(&pool).await?, ("pending".to_owned(), Some(2)));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_redispatch_after_an_unpublished_attempt_uses_the_next_attempt(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;

    let first =
        sqlx::query_file_scalar!("queries/indexing/storage/dispatch_audio.sql", "42", 1_i64)
            .fetch_one(&pool)
            .await?;
    let retried =
        sqlx::query_file_scalar!("queries/indexing/storage/dispatch_audio.sql", "42", 1_i64)
            .fetch_one(&pool)
            .await?;
    sqlx::query(
        "UPDATE audio_index_wire_state
         SET status = 'quarantined', quarantine_reason = 'dispatch_publish_failed'",
    )
    .execute(&pool)
    .await?;
    let after_quarantine =
        sqlx::query_file_scalar!("queries/indexing/storage/dispatch_audio.sql", "42", 1_i64)
            .fetch_one(&pool)
            .await?;
    sqlx::query("UPDATE storage_event_state SET uploaded_generation = 2")
        .execute(&pool)
        .await?;
    let next_generation =
        sqlx::query_file_scalar!("queries/indexing/storage/dispatch_audio.sql", "42", 2_i64)
            .fetch_one(&pool)
            .await?;

    assert_eq!(first, Some(1));
    assert_eq!(retried, Some(1));
    assert_eq!(after_quarantine, Some(2));
    assert_eq!(next_generation, Some(1));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_ok_for_an_attempt_whose_publish_looked_failed_is_still_applied(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    sqlx::query_file!(
        "queries/indexing/storage/abandon_audio_dispatch.sql",
        "42",
        1_i64,
        1_i32
    )
    .execute(&pool)
    .await?;
    let abandoned = wire_outcome(&pool).await?;
    let lease_id = Uuid::now_v7();
    let claimed = claim(&pool, 1, 1, lease_id).await?;
    let committed = commit(&pool, 1, 1, lease_id).await?;

    assert_eq!(
        abandoned,
        (
            "reopenable".to_owned(),
            1,
            1,
            Some("dispatch_publish_failed".to_owned())
        )
    );
    assert!(claimed.claimed);
    assert!(committed.committed && committed.indexed);
    assert_eq!(wire_outcome(&pool).await?, ("done".to_owned(), 1, 4, None));
    assert_eq!(track_state(&pool).await?, ("indexed".to_owned(), true));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unpublished_attempt_is_reissued_with_the_next_attempt_after_the_cooldown(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    sqlx::query_file!(
        "queries/indexing/storage/abandon_audio_dispatch.sql",
        "42",
        1_i64,
        1_i32
    )
    .execute(&pool)
    .await?;
    age_wire(&pool).await?;

    assert_eq!(reopen(&pool, 8).await?, vec![("42".to_owned(), 1, 2)]);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_slow_ok_outlived_by_a_failure_of_its_attempt_is_retried_not_quarantined(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let handler = AudioIndexResultHandler::new(pool.clone(), unreachable_qdrant()?);
    let lease_id = Uuid::now_v7();

    claim(&pool, 1, 1, lease_id).await?;
    handler
        .finish(
            settled(WorkerStatus::Failed, WorkerReason::DeadlineExceeded),
            delivery(2),
        )
        .await?;
    sqlx::query(
        "UPDATE audio_index_wire_state
         SET result_lease_expires_at = now() - interval '1 second'",
    )
    .execute(&pool)
    .await?;
    let late = commit(&pool, 1, 1, lease_id).await?;
    let retry_lease = Uuid::now_v7();
    let retried = claim(&pool, 1, 1, retry_lease).await?;
    let committed = commit(&pool, 1, 1, retry_lease).await?;

    assert!(!late.committed && !late.invalidated && !late.demoted && late.released);
    assert!(retried.claimed);
    assert!(committed.committed && committed.indexed);
    assert_eq!(wire_outcome(&pool).await?, ("done".to_owned(), 1, 4, None));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_outcome_of_equal_rank_does_not_replace_the_recorded_one(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let handler = AudioIndexResultHandler::new(pool.clone(), unreachable_qdrant()?);

    handler
        .finish(
            settled(WorkerStatus::Failed, WorkerReason::UndecodableAudio),
            delivery(1),
        )
        .await?;
    handler
        .finish(
            settled(WorkerStatus::Missing, WorkerReason::AudioForbidden),
            delivery(2),
        )
        .await?;
    age_wire(&pool).await?;
    let terminal = wire_outcome(&pool).await?;
    let reopened = reopen(&pool, 8).await?;

    assert_eq!(
        terminal,
        (
            "terminal".to_owned(),
            1,
            3,
            Some("undecodable_audio".to_owned())
        )
    );
    assert!(reopened.is_empty());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_repeated_reopenable_outcome_does_not_push_the_reopen_back(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let handler = AudioIndexResultHandler::new(pool.clone(), unreachable_qdrant()?);

    handler
        .finish(
            settled(WorkerStatus::Failed, WorkerReason::EngineRestarted),
            delivery(1),
        )
        .await?;
    age_wire(&pool).await?;
    handler.apply_worker_lost(9, &lost_task(1, 1)?).await?;

    assert_eq!(reopen(&pool, 8).await?, vec![("42".to_owned(), 1, 2)]);
    Ok(())
}

async fn settle_unreopenable(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_file_scalar!(
        "queries/indexing/settle_unreopenable_dispatches.sql",
        10_i64,
        3_600_i64,
        8_i32
    )
    .fetch_one(pool)
    .await?)
}

async fn quarantine_reason(pool: &PgPool) -> anyhow::Result<Option<String>> {
    Ok(sqlx::query_scalar(
        "SELECT quarantine_reason FROM audio_index_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrations = false)]
async fn an_attempt_that_used_up_its_reopens_is_quarantined_and_the_track_fails(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let handler = AudioIndexResultHandler::new(pool.clone(), unreachable_qdrant()?);
    sqlx::query("UPDATE audio_index_wire_state SET attempt = 8")
        .execute(&pool)
        .await?;
    handler.apply_worker_lost(9, &lost_task(1, 8)?).await?;

    let during_cooldown = settle_unreopenable(&pool).await?;
    age_wire(&pool).await?;
    let settled = settle_unreopenable(&pool).await?;

    assert_eq!(during_cooldown, 0);
    assert_eq!(settled, 1);
    assert_eq!(wire_state(&pool).await?.0, "quarantined");
    assert_eq!(
        quarantine_reason(&pool).await?.as_deref(),
        Some("reopen_attempts_exhausted")
    );
    assert_eq!(track_state(&pool).await?, ("failed".to_owned(), false));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_reopenable_attempt_of_a_replaced_upload_is_quarantined_without_failing_the_track(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    seed_track(&pool, "pending", 1).await?;
    dispatch(&pool, 1).await?;
    let handler = AudioIndexResultHandler::new(pool.clone(), unreachable_qdrant()?);
    handler.apply_worker_lost(9, &lost_task(1, 1)?).await?;
    sqlx::query("UPDATE storage_event_state SET uploaded_generation = 2")
        .execute(&pool)
        .await?;
    age_wire(&pool).await?;

    assert_eq!(settle_unreopenable(&pool).await?, 1);
    assert_eq!(
        quarantine_reason(&pool).await?.as_deref(),
        Some("reopen_superseded")
    );
    assert_eq!(track_state(&pool).await?, ("pending".to_owned(), false));
    Ok(())
}
