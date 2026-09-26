use backend_contracts::{JobKind, LyricsEmbedPayload, Versioned};
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
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn if_absent_ingress_preserves_existing_work(pool: PgPool) -> anyhow::Result<()> {
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
    assert_eq!(state, (existing.id, existing.payload, 2, 1, 0, 4));
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
