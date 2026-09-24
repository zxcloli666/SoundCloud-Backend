#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;

use std::sync::Arc;

use anyhow::{Context, ensure};
use backend_contracts::pipeline::{
    AudioIndexResult, EncodeResult, LyricsEmbeddingResult, StorageTrackRejected,
    StorageTrackUploaded, TranscriptionResult,
};
use backend_contracts::{ImpressionBatch, JobCommand};
use chrono::{DateTime, Utc};
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::bus::advisory::AdvisoryReaper;
use crate::bus::{Bus, BusConsumers};
use crate::config::JobsConfig;
use crate::db::Databases;
use crate::handlers::taste::{TasteHandler, TasteResult};
use crate::handlers::{CORE_BULK_KINDS, CORE_FAST_KINDS, JobHandlers, OPS_KINDS, accepts_ingress};
use crate::health::HealthState;
use crate::queue::{JobError, JobRepository, NewJob, QueueError, QueueWorker, recover_exhausted};
use crate::scheduler::Scheduler;
use crate::supervisor::Supervisor;

pub async fn run() -> anyhow::Result<()> {
    let config = JobsConfig::from_env().context("jobs configuration is invalid")?;
    info!(instance_id = %config.instance_id, "jobs starting");
    info!(
        embed_lyrics = config.worker_dispatch.embed_lyrics,
        index_audio = config.worker_dispatch.index_audio,
        transcribe = config.worker_dispatch.transcribe,
        taste = config.taste.dispatch,
        lyrics_align_rejected_retry_days = config.worker_dispatch.lyrics_align_rejected_retry_days,
        "worker dispatch switches"
    );

    let databases = Databases::connect(&config)
        .await
        .context("jobs database connection failed")?;
    validate_schema(&databases).await?;

    let qdrant = crate::qdrant::QdrantProvisioner::connect(&config.qdrant)
        .context("Qdrant client initialization failed")?;
    qdrant
        .provision()
        .await
        .context("Qdrant provisioning failed")?;

    let bus = Bus::connect(&config.nats, &config.instance_id).await?;
    let BusConsumers {
        job_ingress,
        impressions,
        collab_results,
        audio_index_results,
        encode_results,
        lyrics_embeddings,
        transcription_results,
        storage_rejections,
        storage_uploads,
    } = bus.provision(&config.nats).await?;
    let taste_results = bus.taste_results(&config.nats).await?;
    let taste = Arc::new(TasteHandler::new(
        databases.main.bulk.clone(),
        bus.clone(),
        qdrant.clone(),
        config.taste.clone(),
    ));

    let handlers = Arc::new(
        JobHandlers::new(
            &databases,
            &config,
            bus.clone(),
            qdrant.clone(),
            connect_call_relay().await,
        )
        .context("jobs handlers could not be created")?,
    );
    handlers
        .bootstrap()
        .await
        .context("jobs bootstrap failed")?;

    let scheduler = Scheduler::configured(databases.main.fast.clone(), &config.schedules);
    scheduler
        .register()
        .await
        .context("jobs schedules could not be registered")?;

    let core_fast_repository = JobRepository::new(
        databases.main.fast.clone(),
        format!("{}:core-fast", config.instance_id),
    );
    let core_bulk_repository = JobRepository::new(
        databases.main.fast.clone(),
        format!("{}:core-bulk", config.instance_id),
    );
    let ops_repository = JobRepository::new(
        databases.main.fast.clone(),
        format!("{}:ops", config.instance_id),
    );
    let ingress_repository = JobRepository::new(
        databases.main.fast.clone(),
        format!("{}:ingress", config.instance_id),
    );
    let core_fast_worker = QueueWorker::new(
        core_fast_repository.clone(),
        handlers.clone(),
        config.queue.clone(),
        config.queue.core_fast.clone(),
        CORE_FAST_KINDS,
    );
    let core_bulk_worker = QueueWorker::new(
        core_bulk_repository,
        handlers.clone(),
        config.queue.clone(),
        config.queue.core_bulk.clone(),
        CORE_BULK_KINDS,
    );
    let ops_worker = QueueWorker::new(
        ops_repository,
        handlers.clone(),
        config.queue.clone(),
        config.queue.ops.clone(),
        OPS_KINDS,
    );
    let cancellation = CancellationToken::new();
    crate::metrics::init();
    let health = HealthState::new()
        .with_metrics_pool(databases.main.fast.clone())
        .require_worker_lanes(&config.worker_dispatch.required_worker_lanes());
    let mut supervisor = Supervisor::new(cancellation.clone(), config.shutdown_grace);

    supervisor.spawn(
        "health server",
        crate::health::serve(config.health_bind, health.clone(), cancellation.clone()),
    );
    supervisor.spawn(
        "worker lost reaper",
        AdvisoryReaper::new(bus.clone(), handlers.clone()).run(cancellation.clone()),
    );
    supervisor.spawn(
        "dependency health monitor",
        crate::health::monitor_dependencies(
            databases.clone(),
            bus,
            qdrant,
            health.clone(),
            cancellation.clone(),
        ),
    );
    supervisor.spawn(
        "API job ingress",
        job_ingress.run(cancellation.clone(), move |command: JobCommand| {
            let repository = ingress_repository.clone();
            async move { accept_job_command(&repository, command).await }
        }),
    );
    let collab_handlers = handlers.clone();
    supervisor.spawn(
        "collab result consumer",
        collab_results.run_raw(cancellation.clone(), move |result| {
            let handlers = collab_handlers.clone();
            async move { handlers.finish_collab(result).await }
        }),
    );
    let taste_receiver = taste.clone();
    supervisor.spawn(
        "taste result consumer",
        taste_results.run_raw(cancellation.clone(), move |result: TasteResult| {
            let taste = taste_receiver.clone();
            async move { taste.finish(result).await }
        }),
    );
    supervisor.spawn("taste schedule", taste.run(cancellation.clone()));
    let lyrics_handlers = handlers.clone();
    supervisor.spawn(
        "lyrics embedding result consumer",
        lyrics_embeddings.run_raw_with_context(
            cancellation.clone(),
            move |result: LyricsEmbeddingResult, delivery| {
                let handlers = lyrics_handlers.clone();
                async move { handlers.finish_lyrics_embedding(result, delivery).await }
            },
        ),
    );
    let indexing_handlers = handlers.clone();
    supervisor.spawn(
        "audio index result consumer",
        audio_index_results.run_raw_with_context(
            cancellation.clone(),
            move |result: AudioIndexResult, delivery| {
                let handlers = indexing_handlers.clone();
                async move { handlers.finish_audio_index(result, delivery).await }
            },
        ),
    );
    let encode_handlers = handlers.clone();
    supervisor.spawn(
        "encode result consumer",
        encode_results.run_raw_with_context(
            cancellation.clone(),
            move |result: EncodeResult, delivery| {
                let handlers = encode_handlers.clone();
                async move { handlers.finish_encode(result, delivery).await }
            },
        ),
    );
    let transcription_handlers = handlers.clone();
    supervisor.spawn(
        "transcription result consumer",
        transcription_results.run_raw_with_context(
            cancellation.clone(),
            move |result: TranscriptionResult, delivery| {
                let handlers = transcription_handlers.clone();
                async move { handlers.finish_transcription(result, delivery).await }
            },
        ),
    );
    let storage_handlers = handlers.clone();
    supervisor.spawn(
        "storage rejection consumer",
        storage_rejections.run_raw_with_context(
            cancellation.clone(),
            move |rejection: StorageTrackRejected, delivery| {
                let handlers = storage_handlers.clone();
                async move { handlers.reject_storage(rejection, delivery).await }
            },
        ),
    );
    let storage_handlers = handlers.clone();
    supervisor.spawn(
        "storage upload consumer",
        storage_uploads.run_raw_with_context(
            cancellation.clone(),
            move |upload: StorageTrackUploaded, delivery| {
                let handlers = storage_handlers.clone();
                async move { handlers.accept_storage_upload(upload, delivery).await }
            },
        ),
    );
    supervisor.spawn(
        "impression consumer",
        impressions.run(cancellation.clone(), move |batch: ImpressionBatch| {
            let handlers = handlers.clone();
            async move { handlers.record_impressions(batch).await }
        }),
    );
    supervisor.spawn(
        "scheduler",
        scheduler.run(cancellation.clone(), health.clone()),
    );
    supervisor.spawn(
        "queue recovery",
        recover_exhausted(core_fast_repository, cancellation.clone()),
    );
    supervisor.spawn(
        "core fast worker",
        core_fast_worker.run(cancellation.clone()),
    );
    supervisor.spawn(
        "core bulk worker",
        core_bulk_worker.run(cancellation.clone()),
    );
    supervisor.spawn("ops worker", ops_worker.run(cancellation.clone()));

    health.mark_ready();
    let result = supervisor.run_until_signal().await;
    health.mark_stopping();
    databases.close().await;
    result.context("jobs runtime failed")?;
    info!(instance_id = %config.instance_id, "jobs stopped");
    Ok(())
}

pub(crate) async fn accept_job_command(
    repository: &JobRepository,
    command: JobCommand,
) -> Result<(), JobError> {
    if !accepts_ingress(command.kind) {
        return Err(JobError::permanent(anyhow::anyhow!(
            "job kind {} is not accepted from API ingress",
            command.kind
        )));
    }
    if command.max_attempts <= 0 {
        return Err(JobError::permanent(anyhow::anyhow!(
            "job max_attempts must be positive"
        )));
    }
    let available_at = DateTime::<Utc>::from_timestamp_millis(command.available_at_unix_ms)
        .ok_or_else(|| JobError::permanent(anyhow::anyhow!("job timestamp is out of range")))?;
    let enqueue_if_absent = command.enqueue_if_absent;
    let job = NewJob {
        id: command.id,
        kind: command.kind,
        dedup_key: command.dedup_key,
        payload: command.payload,
        priority: command.priority,
        max_attempts: command.max_attempts,
        available_at,
    };

    if enqueue_if_absent {
        repository
            .enqueue_if_absent(&job)
            .await
            .map_err(queue_delivery_error)
    } else {
        repository.enqueue(&job).await.map_err(queue_delivery_error)
    }
}

fn queue_delivery_error(error: QueueError) -> JobError {
    match error {
        QueueError::Database(error) => JobError::retryable(error),
        error => JobError::permanent(error),
    }
}

async fn validate_schema(databases: &Databases) -> anyhow::Result<()> {
    let core_ready = sqlx::query_scalar::<_, bool>(
        "SELECT to_regclass('background_jobs') IS NOT NULL
             AND to_regclass('background_job_enqueues') IS NOT NULL
             AND to_regclass('background_job_failures') IS NOT NULL
             AND to_regclass('background_schedules') IS NOT NULL
             AND to_regclass('subscription_snapshot_state') IS NOT NULL",
    )
    .fetch_one(&databases.main.fast)
    .await
    .context("core schema validation failed")?;
    ensure!(
        core_ready,
        "core jobs schema is missing; apply core migrations before starting jobs"
    );
    let core_lanes_ready = sqlx::query_scalar::<_, bool>(
        "SELECT count(*) = 3
         FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name IN (
               'background_jobs',
               'background_job_failures',
               'background_schedules'
           )
           AND column_name = 'lane'
           AND is_nullable = 'NO'",
    )
    .fetch_one(&databases.main.fast)
    .await
    .context("core queue lane validation failed")?;
    let core_indexes_ready = sqlx::query_scalar::<_, bool>(
        "SELECT
            EXISTS (
                SELECT 1
                FROM pg_index
                WHERE indexrelid = to_regclass('background_jobs_claim_idx')
                  AND indisvalid AND indisready
                  AND indnkeyatts = 5
                  AND indpred IS NOT NULL
                  AND pg_get_indexdef(indexrelid, 1, true) = 'lane'
            )
            AND EXISTS (
                SELECT 1
                FROM pg_index
                WHERE indexrelid = to_regclass('background_jobs_oldest_claim_idx')
                  AND indisvalid AND indisready
                  AND indnkeyatts = 5
                  AND indpred IS NOT NULL
                  AND pg_get_indexdef(indexrelid, 1, true) = 'lane'
            )
            AND EXISTS (
                SELECT 1
                FROM pg_index
                WHERE indexrelid = to_regclass('background_jobs_expired_lease_idx')
                  AND indisvalid AND indisready
                  AND indnkeyatts = 4
                  AND indpred IS NOT NULL
                  AND pg_get_indexdef(indexrelid, 1, true) = 'lane'
            )
            AND EXISTS (
                SELECT 1
                FROM pg_index
                WHERE indexrelid = to_regclass('background_job_failures_failed_idx')
                  AND indisvalid AND indisready
                  AND indnkeyatts = 3
                  AND pg_get_indexdef(indexrelid, 1, true) = 'failed_at'
            )",
    )
    .fetch_one(&databases.main.fast)
    .await
    .context("core queue index validation failed")?;
    ensure!(
        core_lanes_ready && core_indexes_ready,
        "core jobs queue schema is incomplete"
    );
    let sync_queue_ready = sqlx::query_scalar::<_, bool>(
        "SELECT to_regclass('sync_queue_head_idx') IS NOT NULL
             AND to_regclass('user_likes_tracks_sync_heal_idx') IS NOT NULL
             AND to_regclass('user_likes_playlists_sync_heal_idx') IS NOT NULL
             AND to_regclass('user_followings_sync_heal_idx') IS NOT NULL
             AND (SELECT count(*) = 6
                  FROM information_schema.columns
                  WHERE table_schema = current_schema()
                    AND table_name = 'sync_queue'
                    AND column_name IN (
                        'generation', 'lease_id', 'lease_generation',
                        'remote_attempted_generation',
                        'remote_completed_generation', 'remote_result'
                    ))
             AND EXISTS (
                 SELECT 1
                 FROM _sqlx_migrations
                 WHERE version = 76 AND success
             )",
    )
    .fetch_one(&databases.main.fast)
    .await
    .context("sync queue schema validation failed")?;
    ensure!(
        sync_queue_ready,
        "sync queue schema is incomplete; apply migrations through 0076"
    );
    let playlist_shadow_ready = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
                 SELECT 1
                 FROM _sqlx_migrations
                 WHERE version = 87 AND success
             )
             AND to_regclass('playlist_track_projection') IS NOT NULL
             AND to_regclass('playlist_remote_snapshots') IS NOT NULL
             AND to_regclass('playlist_remote_snapshot_tracks') IS NOT NULL
             AND to_regclass('playlist_remote_observations') IS NOT NULL
             AND to_regclass('playlist_membership_state') IS NOT NULL
             AND to_regclass('playlist_membership_operations') IS NOT NULL
             AND to_regclass('playlist_reconcile_runs') IS NOT NULL
             AND to_regclass('playlist_legacy_membership_intents') IS NOT NULL
             AND to_regclass('playlist_membership_state_reconcile_due_idx') IS NOT NULL
             AND NOT EXISTS (
                 SELECT 1
                 FROM information_schema.columns
                 WHERE table_schema = current_schema()
                   AND table_name = 'playlists'
                   AND column_name IN ('desired_rev', 'synced_rev', 'tracks_synced_at')
             )",
    )
    .fetch_one(&databases.main.fast)
    .await
    .context("playlist shadow schema validation failed")?;
    ensure!(
        playlist_shadow_ready,
        "playlist shadow schema is incomplete; apply migrations through 0087"
    );
    let oauth_ready = sqlx::query_scalar::<_, bool>(
        "SELECT to_regclass('oauth_app_tokens') IS NOT NULL
             AND to_regclass('oauth_app_token_refresh_state') IS NOT NULL
             AND to_regclass('oauth_app_token_issuance_reservations') IS NOT NULL
             AND to_regclass('oauth_app_request_cooldowns') IS NOT NULL
             AND EXISTS (
                 SELECT 1
                 FROM information_schema.columns
                 WHERE table_schema = current_schema()
                   AND table_name = 'oauth_app_tokens'
                   AND column_name = 'refresh_token'
             )
             AND EXISTS (
                 SELECT 1
                 FROM information_schema.columns
                 WHERE table_schema = current_schema()
                   AND table_name = 'oauth_app_tokens'
                   AND column_name = 'generation'
                   AND is_nullable = 'NO'
             )
             AND (SELECT count(*) = 5
                  FROM information_schema.columns
                  WHERE table_schema = current_schema()
                    AND table_name = 'oauth_app_token_refresh_state'
                    AND column_name IN (
                        'oauth_app_id', 'retry_at', 'lease_id', 'lease_expires_at', 'updated_at'
                    ))
             AND (SELECT count(*) = 5
                  FROM information_schema.columns
                  WHERE table_schema = current_schema()
                    AND table_name = 'oauth_app_token_issuance_reservations'
                    AND column_name IN (
                        'id', 'oauth_app_id', 'client_id', 'reserved_at', 'released_at'
                    ))
             AND to_regclass('oauth_app_token_refresh_state_due_idx') IS NOT NULL
             AND to_regclass('oauth_app_token_issuance_client_window_idx') IS NOT NULL
             AND to_regclass('oauth_app_token_issuance_egress_window_idx') IS NOT NULL",
    )
    .fetch_one(&databases.main.fast)
    .await
    .context("OAuth token schema validation failed")?;
    ensure!(
        oauth_ready,
        "OAuth token coordination schema is incomplete; apply migrations through 0069"
    );
    let recommendation_ready = sqlx::query_scalar::<_, bool>(
        "SELECT to_regclass('user_events_created_type_cover_idx') IS NOT NULL
             AND to_regclass('user_likes_tracks_user_created_key_idx') IS NOT NULL",
    )
    .fetch_one(&databases.main.fast)
    .await
    .context("recommendation jobs schema validation failed")?;
    ensure!(
        recommendation_ready,
        "recommendation jobs schema is incomplete; apply migrations 0013 and 0044"
    );
    let discover_interest_ready = sqlx::query_scalar::<_, bool>(
        "SELECT to_regclass('artists_positive_interest_idx') IS NOT NULL",
    )
    .fetch_one(&databases.main.fast)
    .await
    .context("discover interest schema validation failed")?;
    ensure!(
        discover_interest_ready,
        "discover interest schema is incomplete; apply migration 0065"
    );
    let indexing_ready = sqlx::query_scalar::<_, bool>(
        "SELECT to_regclass('tracks_indexing_stuck_idx') IS NOT NULL
             AND to_regclass('tracks_storage_failed_retry_idx') IS NOT NULL",
    )
    .fetch_one(&databases.main.fast)
    .await
    .context("indexing jobs schema validation failed")?;
    ensure!(
        indexing_ready,
        "indexing jobs schema is incomplete; apply migration 0071"
    );
    let duration_resolver_ready = sqlx::query_scalar::<_, bool>(
        "SELECT (SELECT count(*) = 2
                 FROM information_schema.columns
                 WHERE table_schema = current_schema()
                   AND table_name = 'tracks'
                   AND column_name IN (
                       'duration_resolve_attempts', 'duration_resolve_retry_at'
                   ))
             AND EXISTS (
                 SELECT 1
                 FROM pg_index
                 WHERE indexrelid = to_regclass('tracks_duration_resolve_due_idx')
                   AND indisvalid AND indisready
                   AND indpred IS NOT NULL
             )",
    )
    .fetch_one(&databases.main.fast)
    .await
    .context("duration resolver schema validation failed")?;
    ensure!(
        duration_resolver_ready,
        "duration resolver schema is incomplete; apply migration 0072"
    );
    let pipeline_receipts_ready = sqlx::query_scalar::<_, bool>(
        "SELECT (SELECT count(*) = 5
                 FROM information_schema.columns
                 WHERE table_schema = current_schema()
                   AND table_name = 'pipeline_event_receipts'
                   AND column_name IN (
                       'consumer', 'stream', 'stream_sequence',
                       'event_published_at', 'processed_at'
                   ))
             AND EXISTS (
                 SELECT 1
                 FROM pg_index
                 WHERE indrelid = to_regclass('pipeline_event_receipts')
                   AND indisprimary AND indisvalid AND indisready
                   AND indnkeyatts = 4
                   AND pg_get_indexdef(indexrelid, 1, true) = 'consumer'
                   AND pg_get_indexdef(indexrelid, 2, true) = 'stream'
                   AND pg_get_indexdef(indexrelid, 3, true) = 'stream_sequence'
                   AND pg_get_indexdef(indexrelid, 4, true) = 'event_published_at'
             )
             AND EXISTS (
                 SELECT 1
                 FROM pg_index
                 WHERE indexrelid = to_regclass('pipeline_event_receipts_processed_idx')
                   AND indisvalid AND indisready
                   AND indnkeyatts = 1
                   AND pg_get_indexdef(indexrelid, 1, true) = 'processed_at'
             )",
    )
    .fetch_one(&databases.main.fast)
    .await
    .context("pipeline receipt schema validation failed")?;
    ensure!(
        pipeline_receipts_ready,
        "pipeline receipt schema is incomplete; apply migration 0073"
    );
    let storage_event_state_ready = sqlx::query_scalar::<_, bool>(
        "SELECT (SELECT count(*) = 7
                 FROM information_schema.columns
                 WHERE table_schema = current_schema()
                   AND table_name = 'storage_event_state'
                   AND column_name IN (
                       'sc_track_id', 'stream', 'stream_sequence',
                       'event_published_at', 'uploaded_generation',
                       'transcription_generation', 'updated_at'
                   ))
             AND EXISTS (
                 SELECT 1
                 FROM pg_index
                 WHERE indrelid = to_regclass('storage_event_state')
                   AND indisprimary AND indisvalid AND indisready
                   AND indnkeyatts = 1
                   AND pg_get_indexdef(indexrelid, 1, true) = 'sc_track_id'
             )",
    )
    .fetch_one(&databases.main.fast)
    .await
    .context("storage event state schema validation failed")?;
    ensure!(
        storage_event_state_ready,
        "storage event state schema is incomplete; apply migration 0073"
    );
    let transcription_wire_ready = sqlx::query_scalar::<_, bool>(
        "SELECT (SELECT count(*) = 9
                 FROM information_schema.columns
                 WHERE table_schema = current_schema()
                   AND table_name = 'transcription_wire_state'
                   AND column_name IN (
                       'sc_track_id', 'status', 'upload_generation',
                       'dispatched_at', 'completed_at', 'quarantine_reason',
                       'result_stream_sequence', 'result_published_at', 'updated_at'
                   ))
             AND EXISTS (
                 SELECT 1
                 FROM pg_index
                 WHERE indrelid = to_regclass('transcription_wire_state')
                   AND indisprimary AND indisvalid AND indisready
                   AND indnkeyatts = 1
                   AND pg_get_indexdef(indexrelid, 1, true) = 'sc_track_id'
             )",
    )
    .fetch_one(&databases.main.fast)
    .await
    .context("transcription wire schema validation failed")?;
    ensure!(
        transcription_wire_ready,
        "transcription wire schema is incomplete; apply migration 0074"
    );
    let lyrics_recovery_ready = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
                 SELECT 1
                 FROM information_schema.columns
                 WHERE table_schema = current_schema()
                   AND table_name = 'lyrics_cache'
                   AND column_name = 'embedding_state'
             )
             AND (SELECT count(*) = 22
                 FROM information_schema.columns
                 WHERE table_schema = current_schema()
                   AND table_name = 'lyrics_embedding_wire_state'
                   AND column_name IN (
                       'sc_track_id', 'status', 'lyrics_created_at',
                       'request_version', 'request_text',
                       'request_language', 'request_sha256', 'request_message_id',
                       'first_publish_attempt_at', 'publish_retry_until',
                       'publish_acknowledged_at', 'completed_at', 'quarantine_reason',
                       'result_consumer', 'result_stream', 'result_kind',
                       'result_lease_id', 'result_lease_expires_at',
                       'result_stream_sequence', 'result_published_at',
                       'created_at', 'updated_at'
                   ))
             AND EXISTS (
                 SELECT 1
                 FROM pg_index
                 WHERE indrelid = to_regclass('lyrics_embedding_wire_state')
                   AND indisprimary AND indisvalid AND indisready
                   AND indnkeyatts = 1
                   AND pg_get_indexdef(indexrelid, 1, true) = 'sc_track_id'
             )
             AND EXISTS (
                 SELECT 1 FROM pg_index
                 WHERE indexrelid = to_regclass('storage_event_transcription_reap_idx')
                   AND indisvalid AND indisready
             )
             AND EXISTS (
                 SELECT 1 FROM pg_index
                 WHERE indexrelid = to_regclass('lyrics_cache_embedding_reap_idx')
                   AND indisvalid AND indisready
             )
             AND EXISTS (
                 SELECT 1 FROM pg_index
                 WHERE indexrelid = to_regclass('lyrics_embedding_pending_idx')
                   AND indisvalid AND indisready
             )
             AND EXISTS (
                 SELECT 1 FROM pg_index
                 WHERE indexrelid = to_regclass('transcription_wire_pending_idx')
                   AND indisvalid AND indisready
             )",
    )
    .fetch_one(&databases.main.fast)
    .await
    .context("lyrics recovery schema validation failed")?;
    ensure!(
        lyrics_recovery_ready,
        "lyrics recovery schema is incomplete; apply migration 0075"
    );
    let lyrics_lookup_ready = sqlx::query_scalar::<_, bool>(
        "SELECT (SELECT count(*) = 32
                 FROM information_schema.columns
                 WHERE table_schema = current_schema()
                   AND table_name = 'lyrics_lookup_state')
             AND (SELECT count(*) = 7
                  FROM information_schema.columns
                  WHERE table_schema = current_schema()
                    AND table_name = 'lyrics_lookup_backfill_state')
             AND EXISTS (
                 SELECT 1 FROM pg_index
                 WHERE indexrelid = to_regclass('lyrics_lookup_due_priority_idx')
                   AND indisvalid AND indisready
             )
             AND EXISTS (
                 SELECT 1 FROM pg_index
                 WHERE indexrelid = to_regclass('lyrics_lookup_due_oldest_idx')
                   AND indisvalid AND indisready
             )
             AND EXISTS (
                 SELECT 1 FROM pg_index
                 WHERE indexrelid = to_regclass('lyrics_lookup_expired_claim_idx')
                   AND indisvalid AND indisready
             )
             AND EXISTS (
                 SELECT 1 FROM pg_index
                 WHERE indexrelid = to_regclass('lyrics_lookup_wake_idx')
                   AND indisvalid AND indisready
             )
             AND EXISTS (
                 SELECT 1
                 FROM pg_trigger
                 WHERE tgrelid = to_regclass('tracks')
                   AND tgname = 'tracks_lyrics_lookup_state_refresh'
                   AND NOT tgisinternal
             )",
    )
    .fetch_one(&databases.main.fast)
    .await
    .context("lyrics lookup schema validation failed")?;
    ensure!(
        lyrics_lookup_ready,
        "lyrics lookup schema is incomplete; apply migration 0082"
    );

    let ops_columns_ready = sqlx::query_scalar::<_, bool>(
        "SELECT count(*) = 4
         FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND (
               (table_name = 'rec_impressions' AND column_name IN ('event_id', 'request_id'))
               OR (table_name = 'rec_hard_negatives' AND column_name = 'event_id')
               OR (
                   table_name = 'rec_hard_negatives'
                   AND column_name = 'predicted_score'
                   AND is_nullable = 'YES'
               )
           )",
    )
    .fetch_one(&databases.ops.fast)
    .await
    .context("ops schema validation failed")?;
    let ops_indexes_ready = sqlx::query_scalar::<_, bool>(
        "SELECT
            EXISTS (
                SELECT 1
                FROM pg_index
                WHERE indexrelid = to_regclass('rec_impressions_event_id_idx')
                  AND indrelid = to_regclass('rec_impressions')
                  AND indisunique AND indisvalid AND indisready
                  AND indnkeyatts = 1
                  AND indpred IS NOT NULL
                  AND pg_get_indexdef(indexrelid, 1, true) = 'event_id'
            )
            AND EXISTS (
                SELECT 1
                FROM pg_index
                WHERE indexrelid = to_regclass('rec_hard_negatives_event_id_idx')
                  AND indrelid = to_regclass('rec_hard_negatives')
                  AND indisunique AND indisvalid AND indisready
                  AND indnkeyatts = 1
                  AND indpred IS NOT NULL
                  AND pg_get_indexdef(indexrelid, 1, true) = 'event_id'
            )
            AND EXISTS (
                SELECT 1
                FROM pg_index
                WHERE indexrelid = to_regclass('rec_impressions_user_track_shown_idx')
                  AND indrelid = to_regclass('rec_impressions')
                  AND indisvalid AND indisready
                  AND indnkeyatts = 3
                  AND pg_get_indexdef(indexrelid, 1, true) = 'sc_user_id'
                  AND pg_get_indexdef(indexrelid, 2, true) = 'sc_track_id'
                  AND pg_get_indexdef(indexrelid, 3, true) = 'shown_at'
                  AND (indoption[2] & 1) = 1
            )",
    )
    .fetch_one(&databases.ops.fast)
    .await
    .context("ops index validation failed")?;
    ensure!(
        ops_columns_ready && ops_indexes_ready,
        "ops telemetry schema is incomplete"
    );
    Ok(())
}

async fn connect_call_relay() -> Option<std::sync::Arc<call_relay::Client>> {
    let endpoint = std::env::var("CALL_CONTROL_ENDPOINT").ok()?;
    if endpoint.is_empty() {
        return None;
    }
    let relay_secret = std::env::var("CALL_RELAY_SECRET").unwrap_or_default();
    if relay_secret.is_empty() {
        tracing::warn!("CALL_RELAY_SECRET is empty; the relay will reject jobs requests");
    }
    let config = call_relay::Config {
        control_endpoint: Some(endpoint),
        upstream_proxy: None,
        instance_id: format!("jobs-{}", std::process::id()),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        relay_secret,
        policy: call_relay::tiers::Policy {
            order: vec![call_relay::Tier::Client],
            timeout_ms: 15_000,
            fallback_on_status_5xx: true,
        },
    };
    match call_relay::Client::connect(config).await {
        Ok(client) => {
            tracing::info!("call-relay connected");
            Some(std::sync::Arc::new(client))
        }
        Err(error) => {
            tracing::warn!(%error, "call-relay connect failed; external fetches stay direct/proxy");
            None
        }
    }
}
