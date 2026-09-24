use std::path::{Path, PathBuf};

use sqlx::PgPool;

use super::*;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn create() -> std::io::Result<Self> {
        let path = std::env::temp_dir().join(format!("jobs-bootstrap-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _result = std::fs::remove_dir_all(&self.0);
    }
}

async fn install_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!("../../../../api/migrations/0000_initial.sql"))
        .execute(pool)
        .await?;
    sqlx::raw_sql(include_str!(
        "../../../../api/migrations/0058_subscription_snapshot_state.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

fn config(directory: &Path) -> SubscriptionsConfig {
    SubscriptionsConfig {
        snapshot_dir: directory.to_owned(),
        max_file_bytes: 1_024,
        max_entries: 10,
    }
}

async fn save_snapshot(directory: &Path, body: &[u8]) -> std::io::Result<()> {
    tokio::fs::write(directory.join("subscriptions.json"), body).await
}

#[sqlx::test(migrations = false)]
async fn bootstrap_marker_prevents_stale_restore_after_database_becomes_empty(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let directory = TestDirectory::create()?;
    save_snapshot(
        directory.path(),
        br#"[{"user_urn":"soundcloud:users:1","exp_date":100}]"#,
    )
    .await?;
    let handler = SubscriptionSnapshotHandler::new(pool.clone(), &config(directory.path()));

    handler.bootstrap().await?;
    let restored = sqlx::query_file!("queries/subscriptions/test_state.sql")
        .fetch_one(&pool)
        .await?;
    assert_eq!((restored.count, restored.initialized), (1, true));

    sqlx::query_file!("queries/subscriptions/test_clear.sql")
        .execute(&pool)
        .await?;
    save_snapshot(directory.path(), b"invalid JSON").await?;
    let restarted = SubscriptionSnapshotHandler::new(pool.clone(), &config(directory.path()));
    restarted.bootstrap().await?;

    let count = sqlx::query_file_scalar!("queries/subscriptions/test_count.sql")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn invalid_initial_snapshot_fails_without_marking_bootstrap_complete(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_schema(&pool).await?;
    let directory = TestDirectory::create()?;
    save_snapshot(directory.path(), b"invalid JSON").await?;
    let handler = SubscriptionSnapshotHandler::new(pool.clone(), &config(directory.path()));

    assert!(matches!(
        handler.bootstrap().await,
        Err(JobError::Permanent(_))
    ));
    let initialized = sqlx::query_file_scalar!("queries/subscriptions/test_initialized.sql")
        .fetch_one(&pool)
        .await?;
    assert!(!initialized);
    Ok(())
}
