use sqlx::PgPool;

use super::*;

const ARTIST_INTEREST_INSTALL: i64 = 429;

#[sqlx::test(migrations = false)]
async fn a_core_target_stops_after_its_migration_and_a_later_run_finishes(
    pool: PgPool,
) -> anyhow::Result<()> {
    run_core_up_to(&pool, None, Some(ARTIST_INTEREST_INSTALL)).await?;

    assert_eq!(applied_through(&pool).await?, Some(ARTIST_INTEREST_INSTALL));
    assert!(relation_exists(&pool, "artist_interest").await?);
    assert!(interest_score_exists(&pool).await?);

    run_core(&pool, None).await?;

    let newest = CORE.iter().map(|migration| migration.version).max();
    assert_eq!(applied_through(&pool).await?, newest);
    assert!(!interest_score_exists(&pool).await?);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_core_target_below_the_applied_schema_changes_nothing(
    pool: PgPool,
) -> anyhow::Result<()> {
    run_core(&pool, None).await?;
    let before = applied_through(&pool).await?;

    run_core_up_to(&pool, None, Some(ARTIST_INTEREST_INSTALL)).await?;

    assert_eq!(applied_through(&pool).await?, before);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_unknown_core_target_applies_nothing(pool: PgPool) -> anyhow::Result<()> {
    let error = run_core_up_to(&pool, None, Some(4290)).await.err();

    assert!(matches!(
        error,
        Some(MigrationError::UnknownTarget { version: 4290 })
    ));
    assert!(!relation_exists(&pool, "_sqlx_migrations").await?);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_core_target_below_the_connection_prefix_is_refused(pool: PgPool) -> anyhow::Result<()> {
    let error = run_core_up_to(&pool, None, Some(CONNECTION_MIGRATION - 1))
        .await
        .err();

    assert!(matches!(
        error,
        Some(MigrationError::TargetBelowPrefix { .. })
    ));
    assert!(!relation_exists(&pool, "_sqlx_migrations").await?);
    Ok(())
}

async fn applied_through(pool: &PgPool) -> anyhow::Result<Option<i64>> {
    Ok(
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(pool)
            .await?,
    )
}

async fn relation_exists(pool: &PgPool, name: &str) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(name)
        .fetch_one(pool)
        .await?)
}

async fn interest_score_exists(pool: &PgPool) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1
             FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND table_name = 'artists'
               AND column_name = 'interest_score'
         )",
    )
    .fetch_one(pool)
    .await?)
}
