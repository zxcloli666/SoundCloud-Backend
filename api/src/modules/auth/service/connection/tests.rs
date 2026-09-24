use chrono::{DateTime, Utc};
use futures::future::join_all;
use sqlx::PgPool;

use super::*;

const TEST_OAUTH_APP_ID: Uuid = Uuid::from_u128(0xaaaaaaaa_aaaa_4aaa_8aaa_aaaaaaaaaaaa);

#[path = "rejection_tests.rs"]
mod rejection_tests;

async fn install_legacy_schema(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!(
        "../../../../../queries/auth/test/install_legacy_auth_schema.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

async fn migrate_connections(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!(
        "../../../../../migrations/0059_soundcloud_connections.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

async fn migrate_token_rejection(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!(
        "../../../../../migrations/0061_soundcloud_token_rejection.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

async fn remove_environment_oauth_source(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!(
        "../../../../../migrations/0066_remove_environment_oauth_source.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

async fn install_connections(pool: &PgPool) -> anyhow::Result<()> {
    install_legacy_schema(pool).await?;
    sqlx::query("INSERT INTO oauth_apps (id) VALUES ($1)")
        .bind(TEST_OAUTH_APP_ID)
        .execute(pool)
        .await?;
    migrate_connections(pool).await?;
    migrate_token_rejection(pool).await?;
    remove_environment_oauth_source(pool).await?;
    sqlx::raw_sql(include_str!(
        "../../../../../migrations/0089_refresh_rejection_confirmation.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

async fn insert_connection(pool: &PgPool) -> anyhow::Result<(Uuid, Uuid)> {
    let connection_id = Uuid::now_v7();
    let session_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections (
            id, soundcloud_user_id, oauth_app_id,
            access_token, refresh_token, expires_at, scope
         ) VALUES ($1, '42', $2, 'access', 'refresh', now() - interval '1 hour', '')",
    )
    .bind(connection_id)
    .bind(TEST_OAUTH_APP_ID)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(session_id)
        .bind(connection_id)
        .execute(pool)
        .await?;
    Ok((connection_id, session_id))
}

fn connection(expires_at: DateTime<Utc>) -> SoundCloudConnection {
    SoundCloudConnection {
        id: Uuid::nil(),
        soundcloud_user_id: "42".to_owned(),
        oauth_app_id: Some(TEST_OAUTH_APP_ID),
        access_token: "access".to_owned(),
        refresh_token: "refresh".to_owned(),
        expires_at,
        scope: String::new(),
        refresh_generation: 1,
        refresh_failure_count: 0,
        refresh_lease_id: None,
        refresh_lease_expires_at: None,
        last_refresh_attempt_at: None,
        last_refresh_success_at: None,
        last_refresh_error_kind: None,
        last_refresh_error: None,
        retry_at: None,
    }
}

fn auth_session(expires_at: DateTime<Utc>) -> AuthSession {
    AuthSession {
        id: Uuid::now_v7(),
        soundcloud_connection_id: Some(Uuid::now_v7()),
        soundcloud_user_id: Some("42".to_owned()),
        username: Some("listener".to_owned()),
        oauth_app_id: Some(TEST_OAUTH_APP_ID),
        oauth_app_active: Some(true),
        expires_at: Some(expires_at),
        refresh_lease_id: None,
        refresh_lease_expires_at: None,
        last_refresh_attempt_at: None,
        last_refresh_success_at: None,
        last_refresh_error_kind: None,
        last_refresh_error: None,
        retry_at: None,
    }
}

#[test]
fn active_connection_is_ready() {
    let connection = connection(Utc::now() + chrono::Duration::hours(1));

    let response = connection_response(Some(&connection));

    assert!(matches!(response.state, SoundCloudConnectionState::Ready));
    assert!(!response.can_refresh);
}

#[test]
fn expired_connection_remains_refreshable() {
    let connection = connection(Utc::now() - chrono::Duration::days(7));

    let response = connection_response(Some(&connection));

    assert!(response.can_refresh);
}

#[test]
fn invalid_grant_requires_reauthorization_without_removing_connection() {
    let mut connection = connection(Utc::now() - chrono::Duration::hours(1));
    connection.last_refresh_error_kind = Some("reauthorization_required".to_owned());

    let response = connection_response(Some(&connection));

    assert!(matches!(
        response.state,
        SoundCloudConnectionState::ReauthorizationRequired
    ));
    assert!(!response.can_use_soundcloud);
    assert!(!access_token_is_usable(&connection));
}

#[test]
fn rejected_access_token_is_never_reused() {
    let mut connection = connection(Utc::now() + chrono::Duration::hours(1));
    connection.last_refresh_error_kind = Some("token_rejected".to_owned());
    connection.retry_at = Some(Utc::now() + chrono::Duration::minutes(5));

    let response = connection_response(Some(&connection));

    assert!(matches!(
        response.state,
        SoundCloudConnectionState::RetryLater
    ));
    assert!(!response.can_use_soundcloud);
    assert!(!response.can_refresh);
    assert!(!access_token_is_usable(&connection));
}

#[test]
fn refresh_requires_rotated_credentials() {
    let mut token = sc::ScTokenResponse {
        access_token: "access".to_owned(),
        refresh_token: "refresh".to_owned(),
        expires_in: 3_600,
        scope: String::new(),
        token_type: String::new(),
    };

    assert!(refreshed_token_is_complete(&token));
    token.refresh_token.clear();
    assert!(!refreshed_token_is_complete(&token));
}

#[test]
fn stale_retry_error_is_not_exposed_after_cooldown() {
    let mut connection = connection(Utc::now() + chrono::Duration::hours(1));
    connection.last_refresh_error_kind = Some("temporarily_unavailable".to_owned());
    connection.last_refresh_error = Some("old failure".to_owned());
    connection.retry_at = Some(Utc::now() - chrono::Duration::seconds(1));

    let response = connection_response(Some(&connection));

    assert!(matches!(response.state, SoundCloudConnectionState::Ready));
    assert!(response.error_code.is_none());
    assert!(response.error_message.is_none());
    assert!(response.retry_after_sec.is_none());
}

#[test]
fn missing_oauth_source_does_not_require_login() {
    let mut connection = connection(Utc::now() + chrono::Duration::hours(1));
    connection.oauth_app_id = None;

    let response = connection_response(Some(&connection));

    assert!(matches!(
        response.state,
        SoundCloudConnectionState::RetryLater
    ));
    assert!(!response.can_refresh);
    assert!(response.can_use_soundcloud);
}

#[test]
fn inactive_oauth_app_is_not_reported_as_refreshable() {
    let mut session = auth_session(Utc::now() - chrono::Duration::minutes(1));
    session.oauth_app_active = Some(false);

    let response = auth_session_response(&session);

    assert!(matches!(
        response.state,
        SoundCloudConnectionState::RetryLater
    ));
    assert!(!response.can_refresh);
}

#[test]
fn confirmed_rejection_disables_soundcloud_without_removing_the_session() {
    let connection = connection(Utc::now() + chrono::Duration::hours(1));
    let attempt = RefreshAttempt {
        outcome: RefreshOutcome::ReauthorizationRequired,
        connection: Some(connection),
    };

    let response = attempt.response();

    assert!(matches!(
        response.state,
        SoundCloudConnectionState::ReauthorizationRequired
    ));
    assert!(!response.can_use_soundcloud);
    assert!(!response.can_refresh);
    assert_eq!(
        response.error_code.as_deref(),
        Some("reauthorization_required")
    );
}

#[test]
fn retry_delay_grows_and_is_capped() {
    let mut connection = connection(Utc::now());
    connection.id = Uuid::from_u128(42);

    let first = retry_delay(&connection, 30, 900);
    connection.refresh_failure_count = 1;
    let second = retry_delay(&connection, 30, 900);
    connection.refresh_failure_count = 100;
    let capped = retry_delay(&connection, 30, 900);

    assert!((30..=60).contains(&first));
    assert!((60..=120).contains(&second));
    assert_eq!(capped, 900);
}

#[test]
fn retry_after_rounds_up() {
    let now = Utc::now();

    assert_eq!(
        seconds_until(now + chrono::Duration::milliseconds(1_001), now),
        2
    );
}

#[sqlx::test(migrations = false)]
async fn migration_preserves_sessions_and_distinct_grants(pool: PgPool) -> anyhow::Result<()> {
    install_legacy_schema(&pool).await?;
    sqlx::raw_sql(include_str!(
        "../../../../../queries/auth/test/seed_legacy_auth_sessions.sql"
    ))
    .execute(&pool)
    .await?;

    migrate_connections(&pool).await?;
    remove_environment_oauth_source(&pool).await?;

    let session_count: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(&pool)
        .await?;
    let connection_count: i64 = sqlx::query_scalar("SELECT count(*) FROM soundcloud_connections")
        .fetch_one(&pool)
        .await?;
    let links = sqlx::query_as::<_, (Uuid, Option<Uuid>)>(
        "SELECT id, soundcloud_connection_id FROM sessions ORDER BY id",
    )
    .fetch_all(&pool)
    .await?;

    assert_eq!(session_count, 8);
    assert_eq!(connection_count, 5);
    assert_eq!(links[0].1, links[1].1);
    assert_eq!(links[0].1, Some(links[1].0));
    assert_ne!(links[0].1, links[2].1);
    assert_ne!(links[2].1, links[3].1);
    assert!(links[4].1.is_some());
    assert!(links[5].1.is_none());
    assert!(links[6].1.is_none());
    assert!(links[7].1.is_some());

    let missing_app: Option<Uuid> = sqlx::query_scalar(
        "SELECT oauth_app_id
         FROM soundcloud_connections
         WHERE soundcloud_user_id = '88'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(missing_app, None);

    let legacy_columns: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns
         WHERE table_name = 'sessions'
           AND column_name IN (
               'access_token', 'refresh_token', 'expires_at', 'scope',
               'soundcloud_user_id', 'username', 'oauth_app_id'
           )",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(legacy_columns, 0);

    let reaper_enabled: bool = sqlx::query_scalar(
        "SELECT enabled FROM background_schedules WHERE kind = 'auth.reap_sessions'",
    )
    .fetch_one(&pool)
    .await?;
    let reaper_jobs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM background_jobs
         WHERE kind = 'auth.reap_sessions'",
    )
    .fetch_one(&pool)
    .await?;

    assert!(!reaper_enabled);
    assert_eq!(reaper_jobs, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn refresh_lease_has_one_winner_across_connections(pool: PgPool) -> anyhow::Result<()> {
    install_connections(&pool).await?;
    let (connection_id, _) = insert_connection(&pool).await?;

    let claims = (0..50)
        .map(|_| {
            let pool = pool.clone();
            async move {
                sqlx::query_file_as!(
                    SoundCloudConnection,
                    "queries/auth/service/claim_connection_refresh.sql",
                    connection_id,
                    1_i64,
                    "access",
                    Uuid::new_v4(),
                    REFRESH_LEASE_SECONDS
                )
                .fetch_optional(&pool)
                .await
            }
        })
        .collect::<Vec<_>>();
    let claims = join_all(claims)
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    assert_eq!(claims.into_iter().flatten().count(), 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_single_refresh_rejection_does_not_require_login(pool: PgPool) -> anyhow::Result<()> {
    install_connections(&pool).await?;
    let (connection_id, _) = insert_connection(&pool).await?;
    let lease_id = Uuid::new_v4();
    let claimed = sqlx::query_file_as!(
        SoundCloudConnection,
        "queries/auth/service/claim_connection_refresh.sql",
        connection_id,
        1_i64,
        "access",
        lease_id,
        REFRESH_LEASE_SECONDS
    )
    .fetch_one(&pool)
    .await?;
    sqlx::query_file!(
        "queries/auth/service/fail_connection_refresh_reauth.sql",
        connection_id,
        lease_id,
        claimed.refresh_generation,
        "SoundCloud rejected the refresh token"
    )
    .execute(&pool)
    .await?;
    let connection = sqlx::query_file_as!(
        SoundCloudConnection,
        "queries/auth/service/get_connection_by_id.sql",
        connection_id
    )
    .fetch_one(&pool)
    .await?;
    assert!(matches!(
        connection_response(Some(&connection)).state,
        SoundCloudConnectionState::RetryLater
    ));
    assert!(connection.retry_at.is_some());
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn delayed_refresh_snapshot_cannot_rotate_a_newer_token(pool: PgPool) -> anyhow::Result<()> {
    install_connections(&pool).await?;
    let (connection_id, _) = insert_connection(&pool).await?;
    let winner_lease = Uuid::new_v4();
    let claimed = sqlx::query_file_as!(
        SoundCloudConnection,
        "queries/auth/service/claim_connection_refresh.sql",
        connection_id,
        1_i64,
        "access",
        winner_lease,
        REFRESH_LEASE_SECONDS
    )
    .fetch_one(&pool)
    .await?;
    let completed = sqlx::query_file_as!(
        SoundCloudConnection,
        "queries/auth/service/complete_connection_refresh.sql",
        connection_id,
        winner_lease,
        claimed.refresh_generation,
        "new-access",
        "new-refresh",
        Utc::now() + chrono::Duration::hours(1),
        ""
    )
    .fetch_one(&pool)
    .await?;

    let stale = sqlx::query_file_as!(
        SoundCloudConnection,
        "queries/auth/service/claim_connection_refresh.sql",
        connection_id,
        1_i64,
        "access",
        Uuid::new_v4(),
        REFRESH_LEASE_SECONDS
    )
    .fetch_optional(&pool)
    .await?;

    assert!(stale.is_none());
    assert_eq!(completed.access_token, "new-access");
    assert_eq!(completed.refresh_token, "new-refresh");
    assert_eq!(completed.refresh_generation, 3);
    assert_eq!(completed.refresh_lease_id, None);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn stale_refresh_failure_cannot_overwrite_reauthentication(
    pool: PgPool,
) -> anyhow::Result<()> {
    install_connections(&pool).await?;
    let (connection_id, session_id) = insert_connection(&pool).await?;
    let lease_id = Uuid::new_v4();
    let claimed = sqlx::query_file_as!(
        SoundCloudConnection,
        "queries/auth/service/claim_connection_refresh.sql",
        connection_id,
        1_i64,
        "access",
        lease_id,
        REFRESH_LEASE_SECONDS
    )
    .fetch_one(&pool)
    .await?;

    sqlx::query(
        "UPDATE soundcloud_connections
         SET access_token = 'reauth-access',
             refresh_token = 'reauth-refresh',
             refresh_generation = refresh_generation + 1,
             refresh_lease_id = NULL,
             refresh_lease_expires_at = NULL,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(connection_id)
    .execute(&pool)
    .await?;

    let failed = sqlx::query_file!(
        "queries/auth/service/fail_connection_refresh_reauth.sql",
        connection_id,
        lease_id,
        claimed.refresh_generation,
        "invalid_grant"
    )
    .execute(&pool)
    .await?;
    let state = sqlx::query_as::<_, (String, String, Option<String>)>(
        "SELECT access_token, refresh_token, last_refresh_error_kind
         FROM soundcloud_connections
         WHERE id = $1",
    )
    .bind(connection_id)
    .fetch_one(&pool)
    .await?;
    let session_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = $1)")
            .bind(session_id)
            .fetch_one(&pool)
            .await?;

    assert_eq!(failed.rows_affected(), 0);
    assert_eq!(state.0, "reauth-access");
    assert_eq!(state.1, "reauth-refresh");
    assert_eq!(state.2, None);
    assert!(session_exists);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn rejected_token_state_preserves_the_session(pool: PgPool) -> anyhow::Result<()> {
    install_connections(&pool).await?;
    let (connection_id, session_id) = insert_connection(&pool).await?;

    let marked = sqlx::query_file_scalar!(
        "queries/auth/service/mark_connection_token_rejected.sql",
        session_id,
        "access",
        REJECTED_TOKEN_RETRY_SECONDS
    )
    .fetch_optional(&pool)
    .await?;
    let repeated = sqlx::query_file_scalar!(
        "queries/auth/service/mark_connection_token_rejected.sql",
        session_id,
        "access",
        REJECTED_TOKEN_RETRY_SECONDS
    )
    .fetch_optional(&pool)
    .await?;
    let stored = sqlx::query_file_as!(
        SoundCloudConnection,
        "queries/auth/service/get_connection_by_session.sql",
        session_id
    )
    .fetch_one(&pool)
    .await?;
    let session_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sessions WHERE id = $1)")
            .bind(session_id)
            .fetch_one(&pool)
            .await?;
    let response = connection_response(Some(&stored));

    assert!(marked.is_some_and(|seconds| seconds > 0));
    assert!(repeated.is_some_and(|seconds| seconds > 0));
    assert_eq!(stored.id, connection_id);
    assert_eq!(stored.refresh_failure_count, 1);
    assert_eq!(
        stored.last_refresh_error_kind.as_deref(),
        Some("token_rejected")
    );
    assert!(session_exists);
    assert!(matches!(
        response.state,
        SoundCloudConnectionState::RetryLater
    ));
    assert!(!response.can_use_soundcloud);
    assert!(!response.can_refresh);
    Ok(())
}
