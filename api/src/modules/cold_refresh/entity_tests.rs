use axum::http::StatusCode;
use backend_contracts::CatalogEntity;
use sqlx::PgPool;
use uuid::Uuid;

use super::entity::refresh_pending;
use crate::error::AppError;

async fn settled_absent(pool: &PgPool, ago: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO background_job_failures (
             id, kind, lane, dedup_key, payload, priority, generation, attempts, max_attempts,
             last_error, created_at, failed_at
         ) VALUES ($1, 'catalog.refresh', 'core_bulk', 'track:4242:public', '{}', 15, 1, 1, 8,
                   'not_found: SoundCloud API error (status 404)',
                   now() - $2::interval, now() - $2::interval)",
    )
    .bind(Uuid::now_v7())
    .bind(ago)
    .execute(pool)
    .await?;
    Ok(())
}

async fn queued(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(pool)
        .await?)
}

async fn opened(pool: &PgPool) -> AppError {
    refresh_pending(
        pool,
        CatalogEntity::Track,
        "soundcloud:tracks:4242",
        None,
        Some("18"),
        "track_refresh_pending",
        "Track is being loaded",
    )
    .await
}

#[sqlx::test(migrations = "./migrations")]
async fn a_track_soundcloud_has_just_called_absent_is_not_found_without_a_new_refresh(
    pool: PgPool,
) -> anyhow::Result<()> {
    settled_absent(&pool, "3 hours").await?;

    let error = opened(&pool).await;

    assert_eq!(error.status(), StatusCode::NOT_FOUND);
    assert_eq!(queued(&pool).await?, 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_single_absence_is_asked_again_after_a_day(pool: PgPool) -> anyhow::Result<()> {
    settled_absent(&pool, "25 hours").await?;

    let error = opened(&pool).await;

    assert_eq!(error.public_code(), "track_refresh_pending");
    assert_eq!(queued(&pool).await?, 1);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_absence_is_held_as_long_as_it_has_lasted(pool: PgPool) -> anyhow::Result<()> {
    settled_absent(&pool, "10 days").await?;
    settled_absent(&pool, "2 days").await?;

    let error = opened(&pool).await;

    assert_eq!(error.status(), StatusCode::NOT_FOUND);
    assert_eq!(queued(&pool).await?, 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_absence_is_asked_again_once_its_hold_has_run_out(pool: PgPool) -> anyhow::Result<()> {
    settled_absent(&pool, "28 days").await?;
    settled_absent(&pool, "22 days").await?;

    let error = opened(&pool).await;

    assert_eq!(error.public_code(), "track_refresh_pending");
    assert_eq!(queued(&pool).await?, 1);
    Ok(())
}
