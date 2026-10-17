use std::sync::Arc;

use axum::body::to_bytes;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use sqlx::PgPool;
use uuid::Uuid;

use super::refresh::answer;
use crate::modules::auth::handlers::refresh_response;
use crate::modules::auth::{AuthHealthService, AuthService};
use crate::modules::oauth_apps::OAuthAppsService;
use crate::sc::ScClient;

const FRESH: Uuid = Uuid::from_u128(0xa1);
const REJECTED: Uuid = Uuid::from_u128(0xa2);
const FAILING: Uuid = Uuid::from_u128(0xa3);
const THROTTLED: Uuid = Uuid::from_u128(0xa4);
const STALLED: Uuid = Uuid::from_u128(0xa5);
const LEASED: Uuid = Uuid::from_u128(0xa6);
const UNLINKED: Uuid = Uuid::from_u128(0xa7);
const APP_OFF: Uuid = Uuid::from_u128(0xa8);
const APP_OFF_FRESH: Uuid = Uuid::from_u128(0xa9);
const APP_REMOVED: Uuid = Uuid::from_u128(0xaa);
const UNKNOWN: Uuid = Uuid::from_u128(0xff);

#[sqlx::test(migrations = "./migrations")]
async fn every_session_gets_the_old_verdict_here_and_keeps_its_own_on_the_new_path(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let auth = auth(&pool)?;

    for (session, old_path, new_path) in [
        (FRESH, StatusCode::OK, StatusCode::OK),
        (LEASED, StatusCode::ACCEPTED, StatusCode::ACCEPTED),
        (REJECTED, StatusCode::UNAUTHORIZED, StatusCode::CONFLICT),
        (UNLINKED, StatusCode::UNAUTHORIZED, StatusCode::CONFLICT),
        (UNKNOWN, StatusCode::UNAUTHORIZED, StatusCode::UNAUTHORIZED),
        (
            THROTTLED,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::TOO_MANY_REQUESTS,
        ),
        (FAILING, StatusCode::BAD_GATEWAY, StatusCode::BAD_GATEWAY),
        (
            STALLED,
            StatusCode::GATEWAY_TIMEOUT,
            StatusCode::GATEWAY_TIMEOUT,
        ),
    ] {
        let old = answer(session, auth.refresh_soundcloud(session).await)
            .into_response()
            .status();
        let new = auth
            .refresh_soundcloud(session)
            .await
            .map(refresh_response)
            .into_response()
            .status();
        assert_eq!((old, new), (old_path, new_path), "session {session}");
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_login_whose_app_is_off_or_removed_signs_in_again_while_another_app_is_active(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    seed_gone_apps(&pool).await?;
    let auth = auth(&pool)?;

    for (session, old_path, new_path) in [
        (APP_OFF, StatusCode::UNAUTHORIZED, StatusCode::CONFLICT),
        (APP_REMOVED, StatusCode::UNAUTHORIZED, StatusCode::CONFLICT),
        (APP_OFF_FRESH, StatusCode::OK, StatusCode::OK),
    ] {
        let old = answer(session, auth.refresh_soundcloud(session).await).into_response();
        let new = auth
            .refresh_soundcloud(session)
            .await
            .map(refresh_response)
            .into_response();
        assert_eq!(
            (old.status(), new.status()),
            (old_path, new_path),
            "session {session}"
        );
        if old_path == StatusCode::UNAUTHORIZED {
            assert_eq!(
                code(old).await?.as_deref(),
                Some("soundcloud_reauthorization_required"),
                "8.4.13 signs out on this 401 from /auth/refresh, session {session}"
            );
        }
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_login_whose_app_is_off_waits_half_an_hour_when_no_app_is_active(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    seed_gone_apps(&pool).await?;
    sqlx::query("UPDATE oauth_apps SET active = false")
        .execute(&pool)
        .await?;
    let auth = auth(&pool)?;

    for session in [APP_OFF, APP_REMOVED] {
        let old = answer(session, auth.refresh_soundcloud(session).await).into_response();
        let new = auth
            .refresh_soundcloud(session)
            .await
            .map(refresh_response)
            .into_response();
        for response in [old, new] {
            let retry_after = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok());
            assert_eq!(
                (response.status(), retry_after),
                (StatusCode::BAD_GATEWAY, Some("1800")),
                "session {session}"
            );
        }
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_session_left_without_its_soundcloud_account_reads_as_signed_out(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let auth = auth(&pool)?;

    let session = auth.get_auth_session(UNLINKED).await?;

    assert_eq!(
        session.map(|session| (session.soundcloud_connection_id, session.soundcloud_user_id)),
        Some((None, None))
    );
    assert_eq!(
        auth.get_session(UNLINKED)
            .await?
            .map(|session| session.soundcloud_user_id),
        Some(None)
    );
    Ok(())
}

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', 'legacy', 'client', 'secret', 'http://127.0.0.1/');
         INSERT INTO soundcloud_connections
             (id, soundcloud_user_id, oauth_app_id, access_token, refresh_token, expires_at, scope,
              last_refresh_error_kind, retry_at, refresh_lease_id, refresh_lease_expires_at)
         VALUES
             ('00000000-0000-0000-0000-0000000000c1', '17', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
              'a', 'r', now() + interval '1 hour', '', NULL, NULL, NULL, NULL),
             ('00000000-0000-0000-0000-0000000000c2', '18', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
              'a', 'r', now() - interval '1 hour', '', 'reauthorization_required', NULL, NULL, NULL),
             ('00000000-0000-0000-0000-0000000000c3', '19', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
              'a', 'r', now() - interval '1 hour', '', 'temporarily_unavailable',
              now() + interval '2 minutes', NULL, NULL),
             ('00000000-0000-0000-0000-0000000000c4', '20', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
              'a', 'r', now() - interval '1 hour', '', 'rate_limited',
              now() + interval '5 minutes', NULL, NULL),
             ('00000000-0000-0000-0000-0000000000c5', '21', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
              'a', 'r', now() - interval '1 hour', '', 'timed_out',
              now() + interval '1 minute', NULL, NULL),
             ('00000000-0000-0000-0000-0000000000c6', '22', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa',
              'a', 'r', now() - interval '1 hour', '', NULL, NULL,
              '00000000-0000-0000-0000-00000000cafe', now() + interval '10 minutes');
         INSERT INTO sessions (id, soundcloud_connection_id) VALUES
             ('00000000-0000-0000-0000-0000000000a1', '00000000-0000-0000-0000-0000000000c1'),
             ('00000000-0000-0000-0000-0000000000a2', '00000000-0000-0000-0000-0000000000c2'),
             ('00000000-0000-0000-0000-0000000000a3', '00000000-0000-0000-0000-0000000000c3'),
             ('00000000-0000-0000-0000-0000000000a4', '00000000-0000-0000-0000-0000000000c4'),
             ('00000000-0000-0000-0000-0000000000a5', '00000000-0000-0000-0000-0000000000c5'),
             ('00000000-0000-0000-0000-0000000000a6', '00000000-0000-0000-0000-0000000000c6'),
             ('00000000-0000-0000-0000-0000000000a7', NULL);",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed_gone_apps(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri, active)
         VALUES ('bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', 'off', 'off-client', 'secret', 'http://127.0.0.1/', false);
         INSERT INTO soundcloud_connections
             (id, soundcloud_user_id, oauth_app_id, access_token, refresh_token, expires_at, scope)
         VALUES
             ('00000000-0000-0000-0000-0000000000c8', '23', 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
              'a', 'r', now() - interval '1 hour', ''),
             ('00000000-0000-0000-0000-0000000000c9', '24', 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb',
              'a', 'r', now() + interval '1 hour', ''),
             ('00000000-0000-0000-0000-0000000000ca', '25', NULL,
              'a', 'r', now() - interval '1 hour', '');
         INSERT INTO sessions (id, soundcloud_connection_id) VALUES
             ('00000000-0000-0000-0000-0000000000a8', '00000000-0000-0000-0000-0000000000c8'),
             ('00000000-0000-0000-0000-0000000000a9', '00000000-0000-0000-0000-0000000000c9'),
             ('00000000-0000-0000-0000-0000000000aa', '00000000-0000-0000-0000-0000000000ca');",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn code(response: Response) -> anyhow::Result<Option<String>> {
    let body = to_bytes(response.into_body(), 64 * 1024).await?;
    let body: serde_json::Value = serde_json::from_slice(&body)?;
    Ok(body["code"].as_str().map(str::to_owned))
}

fn auth(pool: &PgPool) -> anyhow::Result<Arc<AuthService>> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let sc = ScClient::new(&sc_transport::ScConfig {
        proxy_url: "http://127.0.0.1:1".into(),
        proxy_fallback: false,
        api_base: None,
        home_base: None,
    })?;
    Ok(AuthService::new(
        pool.clone(),
        sc,
        OAuthAppsService::new(pool.clone()),
        AuthHealthService::with_database(redis, pool.clone()),
    ))
}
