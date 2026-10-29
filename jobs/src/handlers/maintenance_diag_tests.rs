use sqlx::PgPool;

use super::MaintenanceHandler;

#[sqlx::test(migrations = false)]
async fn cleanup_removes_only_diag_rows_past_thirty_days(pool: PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!("../../../api/migrations/0156_diag.sql"))
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO diag_bundles (code, at, app_version, os, body)
         SELECT 'SCD-' || lpad(n::text, 8, '0'),
                CASE WHEN n <= 2500 THEN now() - interval '31 days' ELSE now() END,
                '8.4.25', 'windows', '{}'::jsonb
         FROM generate_series(1, 2600) AS n",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO diag_events (at, fingerprint, kind, title, app_version, os, install, body)
         SELECT CASE WHEN n <= 1500 THEN now() - interval '31 days' ELSE now() END,
                'f', 'render', 't', '8.4.25', 'windows', 'i', '{}'::jsonb
         FROM generate_series(1, 1600) AS n",
    )
    .execute(&pool)
    .await?;

    MaintenanceHandler::new(pool.clone()).cleanup_diag().await?;

    let bundles: i64 = sqlx::query_scalar("SELECT count(*) FROM diag_bundles")
        .fetch_one(&pool)
        .await?;
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM diag_events")
        .fetch_one(&pool)
        .await?;
    assert_eq!((bundles, events), (100, 100));
    Ok(())
}
