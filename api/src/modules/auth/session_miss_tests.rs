use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::{NoContext, Timestamp, Uuid};

use super::{Feed, SESSION_IN_FLIGHT, SESSION_POLL, SESSION_WAIT, in_flight};
use crate::modules::auth::{AuthHealthService, AuthService, CoreRpc};
use crate::modules::oauth_apps::OAuthAppsService;
use crate::sc::ScClient;

const MINUTE: Duration = Duration::from_secs(60);

#[test]
fn only_a_v7_id_younger_than_the_window_can_still_be_in_flight() {
    let now = SystemTime::now();

    assert!(!in_flight(Uuid::new_v4(), now));
    assert!(in_flight(Uuid::now_v7(), now));
    assert!(in_flight(
        minted_at(now - (SESSION_IN_FLIGHT - MINUTE)),
        now
    ));
    assert!(!in_flight(
        minted_at(now - (SESSION_IN_FLIGHT + MINUTE)),
        now
    ));
    assert!(in_flight(minted_at(now + MINUTE), now));
    assert!(!in_flight(minted_at(now + SESSION_IN_FLIGHT + MINUTE), now));
}

#[sqlx::test(migrations = "./migrations")]
async fn a_dead_id_answers_at_once_on_a_streaming_feed(pool: PgPool) -> anyhow::Result<()> {
    crate::metrics::init();
    let auth = auth(&pool)?;
    let old = minted_at(SystemTime::now() - (SESSION_IN_FLIGHT + MINUTE));

    for dead in [Uuid::new_v4(), old] {
        let started = Instant::now();
        let found = auth
            .missed(Feed::Streaming, dead, || auth.get_auth_session(dead))
            .await?;
        let waited = started.elapsed();
        assert!(found.is_none(), "session {dead}");
        assert!(waited < SESSION_POLL, "session {dead} waited {waited:?}");
    }
    assert_counted("dead");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_young_v7_miss_asks_for_a_retry_instead_of_a_new_login(
    pool: PgPool,
) -> anyhow::Result<()> {
    crate::metrics::init();
    let auth = auth(&pool)?;
    let young = Uuid::now_v7();

    let started = Instant::now();
    let missed = auth
        .missed(Feed::Streaming, young, || auth.get_auth_session(young))
        .await;
    let waited = started.elapsed();

    let error = match missed {
        Err(error) => error,
        Ok(found) => anyhow::bail!("a young session must not read as signed out: {found:?}"),
    };
    assert!(
        (SESSION_WAIT..Duration::from_millis(1500)).contains(&waited),
        "waited {waited:?}"
    );
    assert_eq!(error.public_code(), "session_not_replicated");
    let response = error.into_response();
    let retry_after = response
        .headers()
        .get(header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok());
    assert_eq!(
        (response.status(), retry_after),
        (StatusCode::SERVICE_UNAVAILABLE, Some("5"))
    );
    assert_counted("in_flight");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_session_that_lands_during_the_wait_is_found(pool: PgPool) -> anyhow::Result<()> {
    crate::metrics::init();
    let auth = auth(&pool)?;
    let young = Uuid::now_v7();
    let landing = {
        let pool = pool.clone();
        tokio::spawn(async move {
            tokio::time::sleep(SESSION_POLL + SESSION_POLL / 5).await;
            store(&pool, young).await
        })
    };

    let found = auth
        .missed(Feed::Streaming, young, || auth.get_auth_session(young))
        .await?;
    landing.await??;

    assert_eq!(
        found.map(|session| (session.id, session.soundcloud_user_id)),
        Some((young, Some("42".to_owned())))
    );
    assert_counted("found_after_wait");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_young_session_logged_out_elsewhere_answers_at_once(pool: PgPool) -> anyhow::Result<()> {
    let auth = auth(&pool)?;
    let young = Uuid::now_v7();
    tombstone(&pool, young).await?;

    let started = Instant::now();
    let found = auth
        .missed(Feed::Streaming, young, || auth.get_auth_session(young))
        .await?;
    let waited = started.elapsed();

    assert!(found.is_none());
    assert!(waited < SESSION_POLL, "waited {waited:?}");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_no_wait_lookup_of_chat_keeps_the_same_age_rule(pool: PgPool) -> anyhow::Result<()> {
    let auth = auth(&pool)?;
    let young = Uuid::now_v7();
    let logged_out = Uuid::now_v7();
    tombstone(&pool, logged_out).await?;
    let old = minted_at(SystemTime::now() - (SESSION_IN_FLIGHT + MINUTE));

    let started = Instant::now();
    let missed = auth.missed_now::<()>(Feed::Streaming, young).await;
    let waited = started.elapsed();

    let error = match missed {
        Err(error) => error,
        Ok(found) => anyhow::bail!("a young session must not ask for a new login: {found:?}"),
    };
    assert_eq!(error.public_code(), "session_not_replicated");
    assert!(waited < SESSION_POLL, "waited {waited:?}");
    for dead in [Uuid::new_v4(), old, logged_out] {
        assert!(
            auth.missed_now::<()>(Feed::Streaming, dead)
                .await?
                .is_none(),
            "session {dead}"
        );
    }
    assert!(auth.missed_now::<()>(Feed::Local, young).await?.is_none());
    assert!(
        auth.missed_now::<()>(Feed::Stalled, Uuid::new_v4())
            .await
            .is_err()
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_split_node_with_replication_switched_off_asks_for_a_retry(
    pool: PgPool,
) -> anyhow::Result<()> {
    assert_eq!(auth(&pool)?.feed().await?, Feed::Local);
    let split = split_mode(&pool).await?;
    let auth = auth(&split)?;
    let young = Uuid::now_v7();

    let feed = auth.feed().await?;
    let started = Instant::now();
    let found = auth.find_auth_session(young).await;
    let waited = started.elapsed();
    split.close().await;

    assert_eq!(feed, Feed::Stalled);
    let error = match found {
        Err(error) => error,
        Ok(found) => anyhow::bail!("a split node without replication answered {found:?}"),
    };
    assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(waited < SESSION_POLL, "waited {waited:?}");
    Ok(())
}

fn minted_at(at: SystemTime) -> Uuid {
    let since = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    Uuid::new_v7(Timestamp::from_unix(
        NoContext,
        since.as_secs(),
        since.subsec_nanos(),
    ))
}

fn assert_counted(outcome: &str) {
    let Some(rendered) = crate::metrics::render_for_tests() else {
        return;
    };
    let series = format!("api_session_miss_total{{outcome=\"{outcome}\"}}");
    assert!(rendered.contains(&series), "{series} missing");
}

async fn tombstone(pool: &PgPool, session_id: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO repl_tombstones (tbl, pk, v)
         VALUES ('sessions', jsonb_build_object('id', $1::uuid), 1)",
    )
    .bind(session_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn store(pool: &PgPool, session_id: Uuid) -> anyhow::Result<()> {
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, access_token, refresh_token, expires_at, scope)
         VALUES ($1, '42', 'access', 'refresh', now() + interval '1 hour', '')",
    )
    .bind(connection)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(session_id)
        .bind(connection)
        .execute(pool)
        .await?;
    Ok(())
}

async fn split_mode(pool: &PgPool) -> anyhow::Result<PgPool> {
    sqlx::query(
        "DO $$ BEGIN EXECUTE format('ALTER DATABASE %I SET scd.mode = %L', current_database(), 'split'); END $$",
    )
    .execute(pool)
    .await?;
    Ok(PgPoolOptions::new()
        .max_connections(2)
        .connect_with((*pool.connect_options()).clone())
        .await?)
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
        CoreRpc::unreachable(),
    ))
}
