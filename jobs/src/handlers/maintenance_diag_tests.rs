use sqlx::PgPool;

use super::MaintenanceHandler;

#[sqlx::test(migrations = false)]
async fn cleanup_removes_diag_rows_past_their_retention(pool: PgPool) -> anyhow::Result<()> {
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
         SELECT CASE WHEN n <= 1500 THEN now() - interval '8 days' ELSE now() END,
                'f', 'render', 't', '8.4.25', 'windows', 'i', '{}'::jsonb
         FROM generate_series(1, 1600) AS n",
    )
    .execute(&pool)
    .await?;

    sqlx::query(
        "INSERT INTO diag_issue_days (fingerprint, day, host, app_version, kind, title, events, first_at, last_at)
         SELECT 'f' || n, (now() AT TIME ZONE 'UTC')::date - CASE WHEN n <= 1500 THEN 31 ELSE 1 END,
                'main', '8.4.25', 'render', 't', 1, now(), now()
         FROM generate_series(1, 1600) AS n",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO diag_issue_origins (fingerprint, day, host, origin)
         SELECT 'f' || n, (now() AT TIME ZONE 'UTC')::date - CASE WHEN n <= 1500 THEN 31 ELSE 1 END,
                'main', 'o'
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
    let days: i64 = sqlx::query_scalar("SELECT count(*) FROM diag_issue_days")
        .fetch_one(&pool)
        .await?;
    let origins: i64 = sqlx::query_scalar("SELECT count(*) FROM diag_issue_origins")
        .fetch_one(&pool)
        .await?;
    assert_eq!((bundles, events, days, origins), (100, 100, 100, 100));
    Ok(())
}
