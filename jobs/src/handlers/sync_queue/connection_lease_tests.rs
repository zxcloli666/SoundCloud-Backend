use chrono::{DateTime, Utc};
use futures::future::join_all;
use sqlx::PgPool;
use uuid::Uuid;

const LEASE_SECONDS: i32 = 60;

async fn connection(pool: &PgPool) -> anyhow::Result<Uuid> {
    let app_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ($1, 'app', 'client', 'secret', 'https://app.test/callback')",
    )
    .bind(app_id)
    .execute(pool)
    .await?;
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections (
             id, soundcloud_user_id, oauth_app_id, access_token, refresh_token, expires_at, scope
         ) VALUES ($1, '42', $2, 'access', 'refresh', now() - interval '1 hour', '')",
    )
    .bind(id)
    .bind(app_id)
    .execute(pool)
    .await?;
    Ok(id)
}

async fn claim(pool: &PgPool, id: Uuid) -> anyhow::Result<Option<(Uuid, i64)>> {
    let (generation, token): (i64, String) = sqlx::query_as(
        "SELECT refresh_generation, access_token FROM soundcloud_connections WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    let lease = Uuid::new_v4();
    let claimed = sqlx::query_file_scalar!(
        "queries/sync_queue/connection/claim_refresh.sql",
        id,
        generation,
        &token,
        lease,
        LEASE_SECONDS
    )
    .fetch_optional(pool)
    .await?;
    Ok(claimed.map(|generation| (lease, generation)))
}

async fn reject(pool: &PgPool, id: Uuid) -> anyhow::Result<(String, Option<DateTime<Utc>>)> {
    let (lease, generation) = claim(pool, id).await?.expect("claim succeeds");
    sqlx::query_file!(
        "queries/sync_queue/connection/fail_reauthorization.sql",
        id,
        lease,
        generation,
        "SoundCloud rejected the refresh token"
    )
    .execute(pool)
    .await?;
    state(pool, id).await
}

async fn state(pool: &PgPool, id: Uuid) -> anyhow::Result<(String, Option<DateTime<Utc>>)> {
    let (kind, retry_at): (Option<String>, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT last_refresh_error_kind, retry_at FROM soundcloud_connections WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?;
    Ok((kind.unwrap_or_default(), retry_at))
}

async fn evidence(pool: &PgPool, id: Uuid) -> anyhow::Result<(i32, Option<DateTime<Utc>>)> {
    Ok(sqlx::query_as(
        "SELECT refresh_rejection_count, first_refresh_rejection_at
         FROM soundcloud_connections WHERE id = $1",
    )
    .bind(id)
    .fetch_one(pool)
    .await?)
}

async fn mark(pool: &PgPool, id: Uuid, seconds: i32) -> anyhow::Result<()> {
    sqlx::query_file!(
        "queries/sync_queue/connection/mark_rejected.sql",
        id,
        "access",
        seconds
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_refresh_lease_has_one_winner(pool: PgPool) -> anyhow::Result<()> {
    let id = connection(&pool).await?;

    let claims = join_all((0..50).map(|_| claim(&pool, id)))
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    assert_eq!(claims.into_iter().flatten().count(), 1);
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_delayed_snapshot_cannot_rotate_a_newer_token(pool: PgPool) -> anyhow::Result<()> {
    let id = connection(&pool).await?;
    let stale_generation: i64 =
        sqlx::query_scalar("SELECT refresh_generation FROM soundcloud_connections WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await?;
    let (lease, generation) = claim(&pool, id).await?.expect("claim succeeds");
    let completed = sqlx::query_file_scalar!(
        "queries/sync_queue/connection/complete_refresh.sql",
        id,
        lease,
        generation,
        "new-access",
        "new-refresh",
        Utc::now() + chrono::Duration::hours(1),
        ""
    )
    .fetch_one(&pool)
    .await?;

    let stale = sqlx::query_file_scalar!(
        "queries/sync_queue/connection/claim_refresh.sql",
        id,
        stale_generation,
        "access",
        Uuid::new_v4(),
        LEASE_SECONDS
    )
    .fetch_optional(&pool)
    .await?;

    assert!(stale.is_none());
    assert_eq!(completed, "new-access");
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_stale_failure_cannot_overwrite_a_new_login(pool: PgPool) -> anyhow::Result<()> {
    let id = connection(&pool).await?;
    let (lease, generation) = claim(&pool, id).await?.expect("claim succeeds");
    sqlx::query(
        "UPDATE soundcloud_connections
         SET access_token = 'login-access', refresh_token = 'login-refresh',
             refresh_generation = refresh_generation + 1,
             refresh_lease_id = NULL, refresh_lease_expires_at = NULL
         WHERE id = $1",
    )
    .bind(id)
    .execute(&pool)
    .await?;

    let failed = sqlx::query_file!(
        "queries/sync_queue/connection/fail_reauthorization.sql",
        id,
        lease,
        generation,
        "invalid_grant"
    )
    .execute(&pool)
    .await?;

    assert_eq!(failed.rows_affected(), 0);
    assert_eq!(state(&pool, id).await?.0, "");
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn recent_rejections_retry_and_only_confirmed_ones_ask_for_login(
    pool: PgPool,
) -> anyhow::Result<()> {
    let id = connection(&pool).await?;
    sqlx::query("UPDATE soundcloud_connections SET created_at = now() - interval '1 day'")
        .execute(&pool)
        .await?;
    for _ in 0..3 {
        let (kind, retry_at) = reject(&pool, id).await?;
        assert_eq!(kind, "temporarily_unavailable");
        assert!(retry_at.is_some());
        sqlx::query("UPDATE soundcloud_connections SET retry_at = now() - interval '1 second'")
            .execute(&pool)
            .await?;
    }

    sqlx::query(
        "UPDATE soundcloud_connections
         SET first_refresh_rejection_at = now() - interval '20 minutes',
             last_refresh_success_at = now() - interval '1 hour'",
    )
    .execute(&pool)
    .await?;
    let (kind, retry_at) = reject(&pool, id).await?;

    assert_eq!(kind, "reauthorization_required");
    assert!(retry_at.is_none());
    assert!(claim(&pool, id).await?.is_none());
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_recent_success_or_a_usable_token_prevents_a_login_request(
    pool: PgPool,
) -> anyhow::Result<()> {
    let id = connection(&pool).await?;
    for (success_minutes_ago, expires_in_minutes) in [(5_i64, -60_i64), (60, 30)] {
        sqlx::query(
            "UPDATE soundcloud_connections
             SET refresh_rejection_count = 2,
                 first_refresh_rejection_at = now() - interval '20 minutes',
                 last_refresh_success_at = now() - $1 * interval '1 minute',
                 expires_at = now() + $2 * interval '1 minute',
                 retry_at = NULL",
        )
        .bind(success_minutes_ago)
        .bind(expires_in_minutes)
        .execute(&pool)
        .await?;
        assert_eq!(reject(&pool, id).await?.0, "temporarily_unavailable");
    }
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_transient_failure_or_a_success_clears_rejection_evidence(
    pool: PgPool,
) -> anyhow::Result<()> {
    let id = connection(&pool).await?;
    let collect = "UPDATE soundcloud_connections
         SET refresh_rejection_count = 2, first_refresh_rejection_at = now() - interval '20 minutes',
             retry_at = NULL";
    sqlx::query(collect).execute(&pool).await?;
    let (lease, generation) = claim(&pool, id).await?.expect("claim succeeds");
    sqlx::query_file!(
        "queries/sync_queue/connection/fail_retryable.sql",
        id,
        lease,
        generation,
        "timed_out",
        "SoundCloud unavailable",
        30_i32
    )
    .execute(&pool)
    .await?;
    assert_eq!(evidence(&pool, id).await?, (0, None));

    sqlx::query(collect).execute(&pool).await?;
    let (lease, generation) = claim(&pool, id).await?.expect("claim succeeds");
    sqlx::query_file_scalar!(
        "queries/sync_queue/connection/complete_refresh.sql",
        id,
        lease,
        generation,
        "new-access",
        "new-refresh",
        Utc::now() + chrono::Duration::hours(1),
        ""
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(evidence(&pool, id).await?, (0, None));
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn an_access_rejection_leaves_a_lease_and_a_confirmed_failure_alone(
    pool: PgPool,
) -> anyhow::Result<()> {
    let id = connection(&pool).await?;
    let (lease, _) = claim(&pool, id).await?.expect("claim succeeds");
    mark(&pool, id, 300).await?;
    let held: (Option<Uuid>, Option<String>) = sqlx::query_as(
        "SELECT refresh_lease_id, last_refresh_error_kind FROM soundcloud_connections WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(held, (Some(lease), None));

    for (kind, retry) in [
        ("reauthorization_required", false),
        ("temporarily_unavailable", true),
    ] {
        sqlx::query(
            "UPDATE soundcloud_connections
             SET refresh_lease_id = NULL, refresh_lease_expires_at = NULL,
                 last_refresh_error_kind = $1,
                 retry_at = CASE WHEN $2 THEN now() + interval '10 minutes' END",
        )
        .bind(kind)
        .bind(retry)
        .execute(&pool)
        .await?;
        let before = state(&pool, id).await?;
        mark(&pool, id, 0).await?;
        let after = state(&pool, id).await?;
        assert_eq!(after.1, before.1);
        if !retry {
            assert_eq!(after.0, kind);
        }
    }
    Ok(())
}

#[sqlx::test(migrations = "../api/migrations")]
async fn a_repeated_access_rejection_counts_once(pool: PgPool) -> anyhow::Result<()> {
    let id = connection(&pool).await?;

    mark(&pool, id, 300).await?;
    mark(&pool, id, 300).await?;

    let (failures, kind): (i32, Option<String>) = sqlx::query_as(
        "SELECT refresh_failure_count, last_refresh_error_kind
         FROM soundcloud_connections WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(failures, 1);
    assert_eq!(kind.as_deref(), Some("token_rejected"));
    Ok(())
}
