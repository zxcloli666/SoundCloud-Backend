use sqlx::PgPool;

use super::MaintenanceHandler;

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn one_cleanup_run_drains_every_expired_auth_request(pool: PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO login_requests (id, state, code_verifier, expires_at)
         SELECT gen_random_uuid(), 'state-' || n, 'verifier',
                timezone('UTC', now()) + CASE WHEN n <= 2500 THEN interval '-1 minute' ELSE interval '15 minutes' END
         FROM generate_series(1, 2600) AS n;
         INSERT INTO link_requests (id, claim_token, mode, expires_at)
         SELECT gen_random_uuid(), 'token-' || n, 'pull',
                timezone('UTC', now()) + CASE WHEN n <= 2500 THEN interval '-1 minute' ELSE interval '5 minutes' END
         FROM generate_series(1, 2600) AS n;",
    )
    .execute(&pool)
    .await?;

    let maintenance = MaintenanceHandler::new(pool.clone());
    maintenance.cleanup_login_requests().await?;
    maintenance.cleanup_link_requests().await?;

    let left: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM login_requests), (SELECT count(*) FROM link_requests)",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(left, (100, 100));
    Ok(())
}
