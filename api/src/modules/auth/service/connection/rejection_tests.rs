use super::*;

#[sqlx::test(migrations = false)]
async fn access_rejection_does_not_change_an_active_refresh_lease(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_connections(&pool).await?;
    let (id, session_id) = insert_connection(&pool).await?;
    let lease = Uuid::new_v4();
    sqlx::query_file_as!(
        SoundCloudConnection,
        "queries/auth/service/claim_connection_refresh.sql",
        id,
        1_i64,
        "access",
        lease,
        REFRESH_LEASE_SECONDS
    )
    .fetch_one(&pool)
    .await?;
    sqlx::query_file_scalar!(
        "queries/auth/service/mark_connection_token_rejected.sql",
        session_id,
        "access",
        300_i32
    )
    .fetch_optional(&pool)
    .await?;
    sqlx::query_file!(
        "../jobs/queries/sync_queue/connection/mark_rejected.sql",
        id,
        "access",
        300_i32
    )
    .execute(&pool)
    .await?;
    let state: (Option<Uuid>, Option<String>, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT refresh_lease_id, last_refresh_error_kind, retry_at FROM soundcloud_connections WHERE id = $1"
    ).bind(id).fetch_one(&pool).await?;
    assert_eq!(state, (Some(lease), None, None));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn access_rejection_cannot_clear_confirmed_failure_or_shorten_backoff(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_connections(&pool).await?;
    let (id, session_id) = insert_connection(&pool).await?;
    for from_jobs in [false, true] {
        for confirmed in [true, false] {
            let kind = if confirmed {
                "reauthorization_required"
            } else {
                "temporarily_unavailable"
            };
            sqlx::query(
                "UPDATE soundcloud_connections SET last_refresh_error_kind = $1,
                retry_at = CASE WHEN $2 THEN NULL ELSE now() + interval '10 minutes' END",
            )
            .bind(kind)
            .bind(confirmed)
            .execute(&pool)
            .await?;
            let before: Option<DateTime<Utc>> =
                sqlx::query_scalar("SELECT retry_at FROM soundcloud_connections WHERE id = $1")
                    .bind(id)
                    .fetch_one(&pool)
                    .await?;
            if from_jobs {
                sqlx::query_file!(
                    "../jobs/queries/sync_queue/connection/mark_rejected.sql",
                    id,
                    "access",
                    0_i32
                )
                .execute(&pool)
                .await?;
            } else {
                sqlx::query_file_scalar!(
                    "queries/auth/service/mark_connection_token_rejected.sql",
                    session_id,
                    "access",
                    300_i32
                )
                .fetch_optional(&pool)
                .await?;
            }
            let state: (String, Option<DateTime<Utc>>) = sqlx::query_as("SELECT last_refresh_error_kind, retry_at FROM soundcloud_connections WHERE id = $1")
                .bind(id).fetch_one(&pool).await?;
            assert_eq!(
                state.1, before,
                "late access rejection changed the refresh retry deadline"
            );
            if confirmed {
                assert_eq!(
                    state.0, kind,
                    "late access rejection cleared confirmed reauthorization"
                );
            }
        }
    }
    Ok(())
}

async fn evidence(pool: &PgPool, id: Uuid) -> anyhow::Result<(i32, Option<DateTime<Utc>>)> {
    Ok(sqlx::query_as("SELECT refresh_rejection_count, first_refresh_rejection_at FROM soundcloud_connections WHERE id = $1")
        .bind(id).fetch_one(pool).await?)
}

#[sqlx::test(migrations = false)]
async fn successful_refresh_from_either_process_clears_rejection_evidence(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_connections(&pool).await?;
    let (id, _) = insert_connection(&pool).await?;
    for from_jobs in [false, true] {
        sqlx::query(
            "UPDATE soundcloud_connections SET refresh_rejection_count = 2,
            first_refresh_rejection_at = now() - interval '20 minutes'",
        )
        .execute(&pool)
        .await?;
        let current = sqlx::query_file_as!(
            SoundCloudConnection,
            "queries/auth/service/get_connection_by_id.sql",
            id
        )
        .fetch_one(&pool)
        .await?;
        let lease = Uuid::new_v4();
        let claimed = sqlx::query_file_as!(
            SoundCloudConnection,
            "queries/auth/service/claim_connection_refresh.sql",
            id,
            current.refresh_generation,
            &current.access_token,
            lease,
            REFRESH_LEASE_SECONDS
        )
        .fetch_one(&pool)
        .await?;
        let expires_at = Utc::now() + chrono::Duration::hours(1);
        if from_jobs {
            sqlx::query_file!(
                "../jobs/queries/sync_queue/connection/complete_refresh.sql",
                id,
                lease,
                claimed.refresh_generation,
                "new-access",
                "new-refresh",
                expires_at,
                ""
            )
            .fetch_one(&pool)
            .await?;
        } else {
            sqlx::query_file!(
                "queries/auth/service/complete_connection_refresh.sql",
                id,
                lease,
                claimed.refresh_generation,
                "new-access",
                "new-refresh",
                expires_at,
                ""
            )
            .fetch_one(&pool)
            .await?;
        }
        assert_eq!(evidence(&pool, id).await?, (0, None));
    }
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn oauth_login_clears_a_confirmed_rejection(pool: PgPool) -> anyhow::Result<()> {
    install_connections(&pool).await?;
    let (id, session_id) = insert_connection(&pool).await?;
    sqlx::query("UPDATE soundcloud_connections SET refresh_rejection_count = 3,
        first_refresh_rejection_at = now() - interval '20 minutes', last_refresh_error_kind = 'reauthorization_required'")
        .execute(&pool).await?;
    sqlx::query_file!(
        "queries/auth/service/update_connection.sql",
        id,
        "42",
        "listener",
        TEST_OAUTH_APP_ID,
        "reauth-access",
        "reauth-refresh",
        Utc::now() + chrono::Duration::hours(1),
        ""
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(evidence(&pool, id).await?, (0, None));
    let session = sqlx::query_file_as!(
        AuthSession,
        "queries/auth/service/get_auth_session.sql",
        session_id
    )
    .fetch_one(&pool)
    .await?;
    assert!(matches!(
        auth_session_response(&session).state,
        SoundCloudConnectionState::Ready
    ));
    Ok(())
}

async fn reject(pool: &PgPool, id: Uuid) -> anyhow::Result<SoundCloudConnection> {
    let current = sqlx::query_file_as!(
        SoundCloudConnection,
        "queries/auth/service/get_connection_by_id.sql",
        id
    )
    .fetch_one(pool)
    .await?;
    let lease = Uuid::new_v4();
    let claimed = sqlx::query_file_as!(
        SoundCloudConnection,
        "queries/auth/service/claim_connection_refresh.sql",
        id,
        current.refresh_generation,
        &current.access_token,
        lease,
        REFRESH_LEASE_SECONDS
    )
    .fetch_one(pool)
    .await?;
    sqlx::query_file!(
        "queries/auth/service/fail_connection_refresh_reauth.sql",
        id,
        lease,
        claimed.refresh_generation,
        "SoundCloud rejected the refresh token"
    )
    .execute(pool)
    .await?;
    Ok(sqlx::query_file_as!(
        SoundCloudConnection,
        "queries/auth/service/get_connection_by_id.sql",
        id
    )
    .fetch_one(pool)
    .await?)
}

#[sqlx::test(migrations = false)]
async fn repeated_recent_rejections_do_not_trigger_login(pool: PgPool) -> anyhow::Result<()> {
    install_connections(&pool).await?;
    let (id, _) = insert_connection(&pool).await?;
    sqlx::query("UPDATE soundcloud_connections SET created_at = now() - interval '1 day'")
        .execute(&pool)
        .await?;
    for _ in 0..4 {
        let connection = reject(&pool, id).await?;
        assert!(matches!(
            connection_response(Some(&connection)).state,
            SoundCloudConnectionState::RetryLater
        ));
        sqlx::query("UPDATE soundcloud_connections SET retry_at = now() - interval '1 second'")
            .execute(&pool)
            .await?;
    }
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn confirmed_rejections_require_login_and_preserve_local_session(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_connections(&pool).await?;
    let (id, session_id) = insert_connection(&pool).await?;
    reject(&pool, id).await?;
    sqlx::query("UPDATE soundcloud_connections SET retry_at = now() - interval '1 second',
        first_refresh_rejection_at = now() - interval '20 minutes', last_refresh_success_at = now() - interval '1 hour'")
        .execute(&pool).await?;
    let second = reject(&pool, id).await?;
    assert!(matches!(
        connection_response(Some(&second)).state,
        SoundCloudConnectionState::RetryLater
    ));
    sqlx::query("UPDATE soundcloud_connections SET retry_at = now() - interval '1 second'")
        .execute(&pool)
        .await?;
    let third = reject(&pool, id).await?;
    assert!(matches!(
        connection_response(Some(&third)).state,
        SoundCloudConnectionState::ReauthorizationRequired
    ));
    assert!(third.retry_at.is_none());
    let session = sqlx::query_file_as!(
        AuthSession,
        "queries/auth/service/get_auth_session.sql",
        session_id
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(session.soundcloud_user_id.as_deref(), Some("42"));
    let claimed = sqlx::query_file_as!(
        SoundCloudConnection,
        "queries/auth/service/claim_connection_refresh.sql",
        id,
        third.refresh_generation,
        "access",
        Uuid::new_v4(),
        REFRESH_LEASE_SECONDS
    )
    .fetch_optional(&pool)
    .await?;
    assert!(claimed.is_none());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn recent_success_or_usable_token_prevents_login_requirement(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_connections(&pool).await?;
    let (id, _) = insert_connection(&pool).await?;
    for (success_minutes_ago, expires_minutes_from_now) in [(5_i64, -60_i64), (60, 30)] {
        sqlx::query(
            "UPDATE soundcloud_connections SET refresh_rejection_count = 2,
            first_refresh_rejection_at = now() - interval '20 minutes',
            last_refresh_success_at = now() - $1 * interval '1 minute',
            expires_at = now() + $2 * interval '1 minute', retry_at = NULL",
        )
        .bind(success_minutes_ago)
        .bind(expires_minutes_from_now)
        .execute(&pool)
        .await?;
        let connection = reject(&pool, id).await?;
        assert!(matches!(
            connection_response(Some(&connection)).state,
            SoundCloudConnectionState::RetryLater
        ));
    }
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn transient_failure_clears_rejection_evidence(pool: PgPool) -> anyhow::Result<()> {
    install_connections(&pool).await?;
    let (id, _) = insert_connection(&pool).await?;
    sqlx::query("UPDATE soundcloud_connections SET refresh_rejection_count = 2,
        first_refresh_rejection_at = now() - interval '20 minutes', last_refresh_success_at = now() - interval '1 hour'")
        .execute(&pool).await?;
    let lease = Uuid::new_v4();
    let claimed = sqlx::query_file_as!(
        SoundCloudConnection,
        "queries/auth/service/claim_connection_refresh.sql",
        id,
        1_i64,
        "access",
        lease,
        REFRESH_LEASE_SECONDS
    )
    .fetch_one(&pool)
    .await?;
    sqlx::query_file!(
        "queries/auth/service/fail_connection_refresh_retryable.sql",
        id,
        lease,
        claimed.refresh_generation,
        "timed_out",
        "SoundCloud unavailable",
        30_i32
    )
    .execute(&pool)
    .await?;
    let evidence: (i32, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT refresh_rejection_count, first_refresh_rejection_at FROM soundcloud_connections WHERE id = $1"
    ).bind(id).fetch_one(&pool).await?;
    assert_eq!(evidence, (0, None));
    sqlx::query("UPDATE soundcloud_connections SET retry_at = NULL")
        .execute(&pool)
        .await?;
    let connection = reject(&pool, id).await?;
    assert!(matches!(
        connection_response(Some(&connection)).state,
        SoundCloudConnectionState::RetryLater
    ));
    Ok(())
}
