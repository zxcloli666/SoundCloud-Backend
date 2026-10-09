use backend_contracts::{JobKind, LyricsEmbedPayload, PlaylistObservePayload, Versioned};
use chrono::{Duration, Utc};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use super::*;

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!(
        "../../api/migrations/0057_background_jobs.sql"
    ))
    .execute(pool)
    .await?;
    sqlx::raw_sql(include_str!(
        "../../api/migrations/0134_background_jobs_maintenance_lane.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn if_absent_ingress_preserves_existing_work_and_raises_its_priority(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let repository = JobRepository::new(pool.clone(), "test".to_owned());
    let existing = NewJob {
        id: Uuid::now_v7(),
        kind: JobKind::LyricsEmbed,
        dedup_key: Some("42".to_owned()),
        payload: json!({ "original": true }),
        priority: 2,
        max_attempts: 4,
        available_at: Utc::now() + Duration::minutes(5),
    };
    repository.enqueue(&existing).await?;
    let command = JobCommand {
        id: Uuid::now_v7(),
        kind: JobKind::LyricsEmbed,
        dedup_key: Some("42".to_owned()),
        enqueue_if_absent: true,
        payload: serde_json::to_value(Versioned::V1(LyricsEmbedPayload {
            sc_track_id: "42".to_owned(),
        }))?,
        priority: 10,
        max_attempts: 8,
        available_at_unix_ms: Utc::now().timestamp_millis(),
    };

    accept_job_command(&repository, command).await?;

    let state = sqlx::query_as::<_, (Uuid, serde_json::Value, i16, i64, i32, i16)>(
        "SELECT id, payload, priority, generation, attempts, max_attempts
         FROM background_jobs
         WHERE kind = 'lyrics.embed' AND dedup_key = '42'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(state, (existing.id, existing.payload, 10, 1, 0, 4));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_viewer_read_lifts_an_observation_the_sweep_queued_behind_bulk_work(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let repository = JobRepository::new(pool.clone(), "test".to_owned());
    let playlist_urn = "soundcloud:playlists:42";
    let payload = serde_json::to_value(Versioned::V1(PlaylistObservePayload {
        playlist_urn: playlist_urn.to_owned(),
    }))?;
    let swept = NewJob {
        id: Uuid::now_v7(),
        kind: JobKind::PlaylistObserveShadow,
        dedup_key: Some(playlist_urn.to_owned()),
        payload: payload.clone(),
        priority: 0,
        max_attempts: 8,
        available_at: Utc::now(),
    };
    repository.enqueue_if_absent(&swept).await?;
    let viewer_read = JobCommand {
        id: Uuid::now_v7(),
        kind: JobKind::PlaylistObserveShadow,
        dedup_key: Some(playlist_urn.to_owned()),
        enqueue_if_absent: true,
        payload,
        priority: 15,
        max_attempts: 8,
        available_at_unix_ms: Utc::now().timestamp_millis(),
    };

    accept_job_command(&repository, viewer_read).await?;

    let state = sqlx::query_as::<_, (Uuid, i16, i64, i32)>(
        "SELECT id, priority, generation, attempts
         FROM background_jobs
         WHERE kind = 'playlists.observe_shadow' AND dedup_key = $1",
    )
    .bind(playlist_urn)
    .fetch_one(&pool)
    .await?;
    assert_eq!(state, (swept.id, 15, 1, 0));
    Ok(())
}

#[tokio::test]
async fn qdrant_tls_client_starts_once_the_crypto_provider_is_installed() -> anyhow::Result<()> {
    tls_common::init_crypto();
    let config = crate::config::QdrantConfig {
        grpc_url: "https://127.0.0.1:9".to_owned(),
        api_key: redact::Secret::new(String::new()),
    };
    let qdrant = crate::qdrant::QdrantProvisioner::connect(&config)?;

    assert!(qdrant.provision().await.is_err());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_invalid_audio_backfill_index_fails_the_indexing_schema_check(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE tracks (sc_track_id text NOT NULL);
         CREATE INDEX tracks_indexing_stuck_idx ON tracks (sc_track_id);
         CREATE INDEX tracks_storage_failed_retry_idx ON tracks (sc_track_id);
         INSERT INTO tracks (sc_track_id) VALUES ('42'), ('42');",
    )
    .execute(&pool)
    .await?;
    let missing = indexing_schema_ready(&pool).await?;

    let failed_build = sqlx::raw_sql(
        "CREATE UNIQUE INDEX CONCURRENTLY tracks_audio_backfill_idx ON tracks (sc_track_id)",
    )
    .execute(&pool)
    .await;
    let invalid = indexing_schema_ready(&pool).await?;

    sqlx::raw_sql("DROP INDEX CONCURRENTLY tracks_audio_backfill_idx")
        .execute(&pool)
        .await?;
    sqlx::raw_sql("CREATE INDEX CONCURRENTLY tracks_audio_backfill_idx ON tracks (sc_track_id)")
        .execute(&pool)
        .await?;
    let rebuilt = indexing_schema_ready(&pool).await?;

    assert!(!missing);
    assert!(failed_build.is_err());
    assert!(!invalid);
    assert!(rebuilt);
    Ok(())
}
