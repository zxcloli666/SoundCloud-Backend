use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use sqlx::{PgConnection, PgPool};
use tokio::sync::Notify;

use super::retry_conflicts;

async fn touch(connection: &mut PgConnection, id: i32) -> sqlx::Result<()> {
    sqlx::query("UPDATE retry_rows SET n = n + 1 WHERE id = $1")
        .bind(id)
        .execute(connection)
        .await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_deadlocked_transaction_runs_again_and_commits_once(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query("CREATE TABLE retry_rows (id int PRIMARY KEY, n int NOT NULL)")
        .execute(&pool)
        .await?;
    sqlx::query("INSERT INTO retry_rows VALUES (1, 0), (2, 0)")
        .execute(&pool)
        .await?;
    let first_held = Arc::new(Notify::new());
    let second_held = Arc::new(Notify::new());
    let other = tokio::spawn({
        let pool = pool.clone();
        let first_held = first_held.clone();
        let second_held = second_held.clone();
        async move {
            let mut tx = pool.begin().await?;
            touch(&mut tx, 1).await?;
            first_held.notify_one();
            second_held.notified().await;
            tokio::time::sleep(Duration::from_millis(300)).await;
            touch(&mut tx, 2).await?;
            tx.commit().await?;
            anyhow::Ok(())
        }
    });
    let attempts = AtomicU32::new(0);
    first_held.notified().await;

    let (pool_ref, attempts_ref, second_held_ref) = (&pool, &attempts, &second_held);
    retry_conflicts(|| async move {
        attempts_ref.fetch_add(1, Ordering::Relaxed);
        let mut tx = pool_ref.begin().await?;
        touch(&mut tx, 2).await?;
        second_held_ref.notify_one();
        touch(&mut tx, 1).await?;
        tx.commit().await?;
        Ok(())
    })
    .await?;
    other.await??;

    assert_eq!(attempts.load(Ordering::Relaxed), 2);
    let counts: Vec<i32> = sqlx::query_scalar("SELECT n FROM retry_rows ORDER BY id")
        .fetch_all(&pool)
        .await?;
    assert_eq!(counts, [2, 2]);
    Ok(())
}
