use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use backend_contracts::pipeline::{LyricsEmbeddingRequest, LyricsEmbeddingResult, Producer};
use backend_contracts::reasons::{WorkerReason, WorkerStatus};
use chrono::{DateTime, TimeZone, Utc};
use futures::future::BoxFuture;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use super::super::vectors::LyricsVectorStore;
use super::*;

const REQUEST_ID: &str = "lyr:42:first";
const TEXT: &str = "first line of the song\nsecond line of the song";

type RecordedPoint = (u64, String, Option<String>, usize);

#[derive(Default)]
struct RecordingStore {
    points: Mutex<Vec<RecordedPoint>>,
}

impl RecordingStore {
    fn points(&self) -> Vec<RecordedPoint> {
        self.points
            .lock()
            .map(|points| points.clone())
            .unwrap_or_default()
    }
}

impl LyricsVectorStore for RecordingStore {
    fn upsert_lyrics<'a>(
        &'a self,
        sc_track_id: u64,
        embedding_request_id: &'a str,
        language: Option<&'a str>,
        vector: &'a [f32],
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            self.points
                .lock()
                .map_err(|_| anyhow::anyhow!("recording store is poisoned"))?
                .push((
                    sc_track_id,
                    embedding_request_id.to_owned(),
                    language.map(str::to_owned),
                    vector.len(),
                ));
            Ok(())
        })
    }
}

struct Fixture {
    pool: PgPool,
    store: Arc<RecordingStore>,
}

impl Fixture {
    async fn new(pool: PgPool) -> anyhow::Result<Self> {
        sqlx::query(
            "INSERT INTO lyrics_cache (
                 sc_track_id, plain_text, source, language, embedding_state
             ) VALUES ('42', $1, 'genius', 'de', 'dispatched')",
        )
        .bind(TEXT)
        .execute(&pool)
        .await?;
        sqlx::query(
            "INSERT INTO lyrics_embedding_wire_state (
                 sc_track_id, status, lyrics_created_at, lyrics_content_generation,
                 request_version, request_text, request_sha256, request_message_id,
                 first_publish_attempt_at, publish_retry_until, publish_acknowledged_at
             )
             SELECT sc_track_id, 'pending', created_at, content_generation,
                    1, $1, $2, $3,
                    to_timestamp(100), to_timestamp(100) + interval '1 day', to_timestamp(101)
             FROM lyrics_cache WHERE sc_track_id = '42'",
        )
        .bind(TEXT)
        .bind(Sha256::digest(TEXT.as_bytes()).to_vec())
        .bind(REQUEST_ID)
        .execute(&pool)
        .await?;
        Ok(Self {
            pool,
            store: Arc::new(RecordingStore::default()),
        })
    }

    fn handler(&self) -> EmbeddingResultHandler {
        EmbeddingResultHandler::new(self.pool.clone(), self.store.clone())
    }

    async fn finish(&self, result: LyricsEmbeddingResult, sequence: u64) -> anyhow::Result<()> {
        self.handler()
            .finish(result, delivery(sequence))
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))
    }

    async fn state(&self) -> anyhow::Result<(String, Option<String>, Option<String>, bool)> {
        Ok(sqlx::query_as(
            "SELECT wire.status, cache.embedding_state, cache.language,
                    cache.embedded_at IS NOT NULL
             FROM lyrics_embedding_wire_state AS wire
             JOIN lyrics_cache AS cache USING (sc_track_id)
             WHERE wire.sc_track_id = '42'",
        )
        .fetch_one(&self.pool)
        .await?)
    }

    async fn receipts(&self) -> anyhow::Result<i64> {
        Ok(
            sqlx::query_scalar("SELECT count(*) FROM pipeline_event_receipts")
                .fetch_one(&self.pool)
                .await?,
        )
    }
}

fn published_at(sequence: u64) -> DateTime<Utc> {
    Utc.timestamp_opt(200 + sequence as i64, 0)
        .single()
        .unwrap_or_default()
}

fn delivery(sequence: u64) -> DeliveryContext {
    DeliveryContext {
        consumer: "backend-done-embed-lyrics".to_owned(),
        stream: "PIPELINE_DONE".to_owned(),
        stream_sequence: sequence,
        delivery_attempt: 1,
        published_at: published_at(sequence),
    }
}

fn result(status: WorkerStatus, reason: Option<WorkerReason>) -> LyricsEmbeddingResult {
    LyricsEmbeddingResult {
        sc_track_id: "42".to_owned(),
        request_id: REQUEST_ID.to_owned(),
        status,
        reason,
        detail: None,
        producer: Producer {
            worker_id: "gpu-main".to_owned(),
            build: "test".to_owned(),
            models: BTreeMap::new(),
            sync_version: None,
        },
        vector: (status == WorkerStatus::Ok).then(|| vec![0.25; 1024]),
        language: Some("EN".to_owned()),
    }
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_vector_is_stored_under_its_request_and_the_lyrics_become_embedded(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;

    fixture.finish(result(WorkerStatus::Ok, None), 1).await?;
    fixture.finish(result(WorkerStatus::Ok, None), 1).await?;

    assert_eq!(
        fixture.store.points(),
        vec![(42, REQUEST_ID.to_owned(), Some("en".to_owned()), 1024)]
    );
    assert_eq!(
        fixture.state().await?,
        (
            "done".to_owned(),
            Some("done".to_owned()),
            Some("en".to_owned()),
            true
        )
    );
    assert_eq!(fixture.receipts().await?, 1);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_result_for_another_request_leaves_the_open_one_alone(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;
    let mut stale = result(WorkerStatus::Ok, None);
    stale.request_id = "lyr:42:older".to_owned();

    fixture.finish(stale, 1).await?;

    assert!(fixture.store.points().is_empty());
    assert_eq!(fixture.state().await?.0, "pending");
    assert_eq!(fixture.receipts().await?, 1);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn replaced_lyrics_quarantine_the_result_and_stay_free_for_a_new_request(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;
    sqlx::query(
        "UPDATE lyrics_cache
         SET plain_text = 'replacement lyrics', content_generation = content_generation + 1,
             embedding_state = NULL
         WHERE sc_track_id = '42'",
    )
    .execute(&fixture.pool)
    .await?;

    fixture.finish(result(WorkerStatus::Ok, None), 1).await?;

    let reason: Option<String> = sqlx::query_scalar(
        "SELECT quarantine_reason FROM lyrics_embedding_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&fixture.pool)
    .await?;
    assert!(fixture.store.points().is_empty());
    assert_eq!(
        fixture.state().await?,
        ("quarantined".to_owned(), None, Some("de".to_owned()), false)
    );
    assert_eq!(reason.as_deref(), Some("request_text_changed"));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn text_that_no_longer_matches_the_request_hash_is_released_for_reembedding(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;
    sqlx::query("UPDATE lyrics_cache SET plain_text = 'edited in place' WHERE sc_track_id = '42'")
        .execute(&fixture.pool)
        .await?;

    fixture.finish(result(WorkerStatus::Ok, None), 1).await?;

    assert!(fixture.store.points().is_empty());
    assert_eq!(
        fixture.state().await?,
        ("quarantined".to_owned(), None, Some("de".to_owned()), false)
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn empty_text_is_skipped_with_the_worker_language(pool: PgPool) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;

    fixture
        .finish(
            result(WorkerStatus::Empty, Some(WorkerReason::EmptyText)),
            1,
        )
        .await?;

    assert_eq!(
        fixture.state().await?,
        (
            "skipped".to_owned(),
            Some("skipped".to_owned()),
            Some("en".to_owned()),
            false
        )
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn an_engine_restart_reopens_the_request_until_the_budget_is_spent(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;

    fixture
        .finish(
            result(WorkerStatus::Failed, Some(WorkerReason::EngineRestarted)),
            1,
        )
        .await?;
    let reopened = fixture.state().await?;
    sqlx::raw_sql(
        "UPDATE lyrics_embedding_wire_state
         SET status = 'pending', completed_at = NULL, reopen_count = 3,
             result_consumer = NULL, result_stream = NULL, result_kind = NULL,
             result_stream_sequence = NULL, result_published_at = NULL
         WHERE sc_track_id = '42';
         UPDATE lyrics_cache SET embedding_state = 'dispatched' WHERE sc_track_id = '42';",
    )
    .execute(&fixture.pool)
    .await?;
    fixture
        .finish(
            result(WorkerStatus::Failed, Some(WorkerReason::EngineRestarted)),
            2,
        )
        .await?;

    assert_eq!(
        reopened,
        ("reopenable".to_owned(), None, Some("de".to_owned()), false)
    );
    assert_eq!(
        fixture.state().await?,
        (
            "failed".to_owned(),
            Some("failed".to_owned()),
            Some("de".to_owned()),
            false
        )
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_terminal_failure_is_recorded_without_a_vector(pool: PgPool) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;

    fixture
        .finish(
            result(
                WorkerStatus::Failed,
                Some(WorkerReason::TextTooLongForModel),
            ),
            1,
        )
        .await?;

    let reason: Option<String> = sqlx::query_scalar(
        "SELECT result_reason FROM lyrics_embedding_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&fixture.pool)
    .await?;
    assert!(fixture.store.points().is_empty());
    assert_eq!(fixture.state().await?.1.as_deref(), Some("failed"));
    assert_eq!(reason.as_deref(), Some("text_too_long_for_model"));
    Ok(())
}

fn lost_request(request_id: &str) -> anyhow::Result<Vec<u8>> {
    Ok(serde_json::to_vec(&LyricsEmbeddingRequest {
        sc_track_id: "42".to_owned(),
        request_id: request_id.to_owned(),
        text: TEXT.to_owned(),
        language: None,
    })?)
}

impl Fixture {
    async fn lose(&self, request_id: &str) -> anyhow::Result<()> {
        self.handler()
            .apply_worker_lost(9, &lost_request(request_id)?)
            .await
            .map_err(|error| anyhow::anyhow!("{error}"))
    }

    async fn result_reason(&self) -> anyhow::Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT result_reason FROM lyrics_embedding_wire_state WHERE sc_track_id = '42'",
        )
        .fetch_one(&self.pool)
        .await?)
    }
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_lost_worker_reopens_the_request_so_the_lyrics_are_embedded_again(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;

    fixture.lose(REQUEST_ID).await?;
    let reopened = fixture.state().await?;
    fixture.lose(REQUEST_ID).await?;
    let lost_twice = fixture.state().await?;
    let reason = fixture.result_reason().await?;
    fixture.finish(result(WorkerStatus::Ok, None), 1).await?;

    assert_eq!(
        reopened,
        ("reopenable".to_owned(), None, Some("de".to_owned()), false)
    );
    assert_eq!(lost_twice, reopened);
    assert_eq!(reason.as_deref(), Some("worker_lost"));
    assert_eq!(fixture.state().await?.0, "done");
    assert_eq!(fixture.store.points().len(), 1);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_lost_worker_fails_the_request_once_the_reopen_budget_is_spent(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;
    sqlx::query("UPDATE lyrics_embedding_wire_state SET reopen_count = 3")
        .execute(&fixture.pool)
        .await?;

    fixture.lose(REQUEST_ID).await?;

    assert_eq!(
        fixture.state().await?,
        (
            "failed".to_owned(),
            Some("failed".to_owned()),
            Some("de".to_owned()),
            false
        )
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_lost_worker_leaves_another_request_and_an_arrived_result_alone(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;

    fixture.lose("lyr:42:older").await?;
    let after_foreign_loss = fixture.state().await?;
    sqlx::query(
        "UPDATE lyrics_embedding_wire_state
         SET result_consumer = 'backend-done-embed-lyrics', result_stream = 'PIPELINE_DONE',
             result_kind = 'vector', result_stream_sequence = 1, result_published_at = now()",
    )
    .execute(&fixture.pool)
    .await?;
    fixture.lose(REQUEST_ID).await?;

    assert_eq!(after_foreign_loss.0, "pending");
    assert_eq!(fixture.state().await?.0, "pending");
    assert_eq!(fixture.result_reason().await?, None);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_lost_worker_closes_a_request_whose_lyrics_were_replaced_and_frees_the_new_lyrics(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;
    sqlx::query(
        "UPDATE lyrics_cache
         SET plain_text = 'replacement lyrics', content_generation = content_generation + 1,
             embedding_state = NULL
         WHERE sc_track_id = '42'",
    )
    .execute(&fixture.pool)
    .await?;

    fixture.lose(REQUEST_ID).await?;

    let reason: Option<String> = sqlx::query_scalar(
        "SELECT quarantine_reason FROM lyrics_embedding_wire_state WHERE sc_track_id = '42'",
    )
    .fetch_one(&fixture.pool)
    .await?;
    assert_eq!(
        fixture.state().await?,
        ("quarantined".to_owned(), None, Some("de".to_owned()), false)
    );
    assert_eq!(reason.as_deref(), Some("lyrics_changed_before_apply"));
    assert_eq!(
        fixture.result_reason().await?.as_deref(),
        Some("worker_lost")
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_lost_task_that_is_not_an_embedding_request_is_refused(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;

    let refused = fixture
        .handler()
        .apply_worker_lost(9, b"{\"sc_track_id\":\"42\"}")
        .await;
    let non_canonical = fixture
        .handler()
        .apply_worker_lost(
            9,
            &serde_json::to_vec(&LyricsEmbeddingRequest {
                sc_track_id: "042".to_owned(),
                request_id: REQUEST_ID.to_owned(),
                text: TEXT.to_owned(),
                language: None,
            })?,
        )
        .await;

    assert!(refused.is_err_and(|error| !error.is_retryable()));
    assert!(non_canonical.is_err_and(|error| !error.is_retryable()));
    assert_eq!(fixture.state().await?.0, "pending");
    Ok(())
}

struct UnavailableStore;

impl LyricsVectorStore for UnavailableStore {
    fn upsert_lyrics<'a>(
        &'a self,
        _sc_track_id: u64,
        _embedding_request_id: &'a str,
        _language: Option<&'a str>,
        _vector: &'a [f32],
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { Err(anyhow::anyhow!("vector store is unavailable")) })
    }
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_vector_waits_while_the_vector_store_is_unavailable(pool: PgPool) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;

    let error = EmbeddingResultHandler::new(fixture.pool.clone(), Arc::new(UnavailableStore))
        .finish(result(WorkerStatus::Ok, None), delivery(1))
        .await
        .expect_err("a vector the store did not take must be retried");

    assert!(error.is_retryable());
    assert_eq!(fixture.state().await?.0, "pending");
    assert_eq!(fixture.receipts().await?, 0);
    Ok(())
}

struct ClaimOutcome {
    settled: bool,
    claimed: bool,
    busy: bool,
    kind_mismatch: bool,
}

async fn claim(
    pool: &PgPool,
    sequence: u64,
    kind: &str,
    lease_id: Uuid,
) -> anyhow::Result<ClaimOutcome> {
    let stream_sequence = i64::try_from(sequence)?;
    let rank: i16 = match kind {
        "vector" => 4,
        "reopen" => 1,
        _ => 3,
    };
    let outcome = sqlx::query_file!(
        "queries/lyrics/claim_embedding_result.sql",
        "backend-done-embed-lyrics",
        "PIPELINE_DONE",
        stream_sequence,
        published_at(sequence),
        "42",
        kind,
        lease_id,
        30_i64,
        REQUEST_ID,
        rank
    )
    .fetch_one(pool)
    .await?;
    Ok(ClaimOutcome {
        settled: outcome.settled,
        claimed: outcome.claimed,
        busy: outcome.busy,
        kind_mismatch: outcome.kind_mismatch,
    })
}

async fn quarantine(pool: &PgPool, sequence: u64, lease_id: Uuid) -> anyhow::Result<(bool, bool)> {
    let stream_sequence = i64::try_from(sequence)?;
    let outcome = sqlx::query_file!(
        "queries/lyrics/quarantine_embedding_result.sql",
        "backend-done-embed-lyrics",
        "PIPELINE_DONE",
        stream_sequence,
        published_at(sequence),
        "42",
        lease_id,
        "test",
        3_i32
    )
    .fetch_one(pool)
    .await?;
    Ok((outcome.owned, outcome.accepted))
}

#[sqlx::test(migrations = "../api/migrations")]
async fn concurrent_results_of_one_rank_have_one_claim_owner(pool: PgPool) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;

    let first = claim(&fixture.pool, 1, "vector", Uuid::now_v7());
    let second = claim(&fixture.pool, 2, "vector", Uuid::now_v7());
    let (first, second) = tokio::try_join!(first, second)?;

    assert_eq!(usize::from(first.claimed) + usize::from(second.claimed), 1);
    assert_eq!(usize::from(first.settled) + usize::from(second.settled), 1);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_vector_takes_the_claim_from_a_weaker_result_and_not_the_reverse(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;

    let skipped = claim(&fixture.pool, 1, "skipped", Uuid::now_v7()).await?;
    let vector = claim(&fixture.pool, 2, "vector", Uuid::now_v7()).await?;
    let late_failure = claim(&fixture.pool, 3, "failed", Uuid::now_v7()).await?;

    assert!(skipped.claimed);
    assert!(vector.claimed && !vector.settled);
    assert!(late_failure.settled && !late_failure.claimed);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_late_vector_replaces_an_earlier_failure_of_the_same_request(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;

    fixture
        .finish(
            result(WorkerStatus::Failed, Some(WorkerReason::DeadlineExceeded)),
            1,
        )
        .await?;
    let failed = fixture.state().await?;
    fixture.finish(result(WorkerStatus::Ok, None), 2).await?;

    assert_eq!(failed.0, "failed");
    assert_eq!(
        fixture.store.points(),
        vec![(42, REQUEST_ID.to_owned(), Some("en".to_owned()), 1024)]
    );
    assert_eq!(
        fixture.state().await?,
        (
            "done".to_owned(),
            Some("done".to_owned()),
            Some("en".to_owned()),
            true
        )
    );
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn an_earlier_failure_does_not_replace_a_vector(pool: PgPool) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;

    fixture.finish(result(WorkerStatus::Ok, None), 1).await?;
    fixture
        .finish(
            result(WorkerStatus::Failed, Some(WorkerReason::DeadlineExceeded)),
            2,
        )
        .await?;

    assert_eq!(fixture.state().await?.0, "done");
    assert_eq!(fixture.receipts().await?, 2);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_result_published_before_the_request_was_recorded_is_still_applied(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;
    sqlx::query(
        "UPDATE lyrics_embedding_wire_state
         SET first_publish_attempt_at = now(), publish_retry_until = now() + interval '1 day'
         WHERE sc_track_id = '42'",
    )
    .execute(&fixture.pool)
    .await?;

    fixture.finish(result(WorkerStatus::Ok, None), 1).await?;

    assert_eq!(fixture.state().await?.0, "done");
    assert_eq!(fixture.store.points().len(), 1);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn active_claim_is_busy_and_expired_claim_is_reclaimed(pool: PgPool) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;
    let first_lease = Uuid::now_v7();

    let first = claim(&fixture.pool, 1, "vector", first_lease).await?;
    let busy = claim(&fixture.pool, 1, "vector", Uuid::now_v7()).await?;
    sqlx::query(
        "UPDATE lyrics_embedding_wire_state
         SET result_lease_expires_at = now() - interval '1 second'
         WHERE sc_track_id = '42'",
    )
    .execute(&fixture.pool)
    .await?;
    let next_lease = Uuid::now_v7();
    let reclaimed = claim(&fixture.pool, 1, "vector", next_lease).await?;
    let stale_quarantine = quarantine(&fixture.pool, 1, first_lease).await?;
    let current_quarantine = quarantine(&fixture.pool, 1, next_lease).await?;

    assert!(first.claimed);
    assert!(busy.busy && !busy.claimed && !busy.settled);
    assert!(reclaimed.claimed);
    assert_eq!(stale_quarantine, (false, false));
    assert_eq!(current_quarantine, (true, true));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn same_delivery_cannot_change_result_kind(pool: PgPool) -> anyhow::Result<()> {
    let fixture = Fixture::new(pool).await?;

    claim(&fixture.pool, 1, "vector", Uuid::now_v7()).await?;
    let changed = claim(&fixture.pool, 1, "skipped", Uuid::now_v7()).await?;

    assert!(changed.kind_mismatch);
    assert!(!changed.claimed);
    assert!(!changed.settled);
    Ok(())
}
