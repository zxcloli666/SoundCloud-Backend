use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use redact::Secret;
use serde_json::{Value, json};
use sqlx::PgPool;
use url::Url;
use uuid::Uuid;

use super::MobileService;
use super::dto::LinkStatus;
use crate::config::{MobileLinkAllow, SoundcloudCfg};
use crate::error::AppError;
use crate::modules::auth::health::AuthHealthService;
use crate::modules::auth::{AuthService, CoreRpc};
use crate::modules::oauth_apps::OAuthAppsService;
use crate::sc::ScClient;

pub(super) const ACCOUNT: &str = "4242";
const OTHER_ACCOUNT: &str = "5151";
const PLAN_HEADER: &str = "sc-mob-userplan";

#[derive(Clone, Copy)]
pub(super) enum Token {
    Pair,
    Captcha,
}

#[derive(Clone, Copy)]
pub(super) enum Graph {
    Reactions,
    Captcha,
    Unauthorized,
}

#[derive(Clone, Copy)]
pub(super) enum Me {
    Account(&'static str),
    Rejected,
}

#[derive(Clone)]
struct FakeSoundCloud {
    token: Token,
    me: Me,
    graph: Graph,
    exchanges: Arc<AtomicUsize>,
}

pub(super) struct Fixture {
    pub(super) mobile: Arc<MobileService>,
    auth: Arc<AuthService>,
    pub(super) session_id: Uuid,
    exchanges: Arc<AtomicUsize>,
}

async fn answer(State(fake): State<FakeSoundCloud>, headers: HeaderMap) -> Response {
    let target = headers
        .get("x-target")
        .and_then(|value| STANDARD.decode(value.as_bytes()).ok())
        .and_then(|value| String::from_utf8(value).ok())
        .and_then(|value| Url::parse(&value).ok());
    match target.as_ref().map(Url::path) {
        Some("/oauth/token") => token(&fake),
        Some("/me") => me(fake.me),
        Some("/graphql") => graphql(fake.graph),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

fn token(fake: &FakeSoundCloud) -> Response {
    let call = fake.exchanges.fetch_add(1, Ordering::SeqCst) + 1;
    match fake.token {
        Token::Pair => axum::Json(json!({
            "access_token": format!("fake-mobile-access-{call}"),
            "refresh_token": format!("fake-mobile-refresh-{call}"),
            "expires_in": 604799,
            "scope": "* fast-connect playcount upload signup purchase"
        }))
        .into_response(),
        Token::Captcha => (
            StatusCode::FORBIDDEN,
            axum::Json(json!({
                "url": "https://geo.captcha-delivery.com/interstitial/?initialCid=fake"
            })),
        )
            .into_response(),
    }
}

fn graphql(graph: Graph) -> Response {
    match graph {
        Graph::Reactions => axum::Json(json!({
            "data": { "targetInteractions": [{
                "targetUrn": "ts:5",
                "userInteraction": "sc:interactiontypevalue:1f525",
                "interactionCounts": [
                    { "interactionTypeValueUrn": "sc:interactiontypevalue:1f525", "count": 3 }
                ]
            }] }
        }))
        .into_response(),
        Graph::Captcha => (
            StatusCode::FORBIDDEN,
            axum::Json(json!({
                "url": "https://geo.captcha-delivery.com/interstitial/?initialCid=fake"
            })),
        )
            .into_response(),
        Graph::Unauthorized => (StatusCode::UNAUTHORIZED, axum::Json(json!({}))).into_response(),
    }
}

fn me(me: Me) -> Response {
    match me {
        Me::Account(account) => {
            ([(PLAN_HEADER, "none")], axum::Json(profile(account))).into_response()
        }
        Me::Rejected => (StatusCode::UNAUTHORIZED, axum::Json(json!({}))).into_response(),
    }
}

fn profile(account: &str) -> Value {
    json!({
        "configuration": { "plan": { "id": "none" } },
        "primary_email_confirmed": true,
        "user": {
            "avatar_url": "https://avatars.example.test/fake-large.jpg",
            "avatar_url_template": "https://avatars.example.test/fake-{size}.jpg",
            "badges": [],
            "created_at": "2020-01-01T00:00:00Z",
            "first_name": "Fake",
            "followers_count": 0,
            "followings_count": 0,
            "is_pro": false,
            "last_name": "Listener",
            "permalink": "fake-listener",
            "station_urns": [],
            "tracks_count": 0,
            "urn": format!("soundcloud:users:{account}"),
            "username": "fake-listener",
            "verified": false,
            "visual_url_template": "https://visuals.example.test/fake-{size}.jpg"
        }
    })
}

async fn fixture(pool: &PgPool, token: Token, me: Me) -> anyhow::Result<Fixture> {
    fixture_graph(pool, token, me, Graph::Reactions).await
}

pub(super) async fn fixture_graph(
    pool: &PgPool,
    token: Token,
    me: Me,
    graph: Graph,
) -> anyhow::Result<Fixture> {
    let exchanges = Arc::new(AtomicUsize::new(0));
    let fake = Router::new().fallback(answer).with_state(FakeSoundCloud {
        token,
        me,
        graph,
        exchanges: exchanges.clone(),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, fake).await });
    let auth = auth(pool)?;
    let soundcloud = SoundcloudCfg {
        proxy_url: format!("http://{address}/"),
        proxy_fallback: false,
        mobile_link: MobileLinkAllow::Everyone,
    };
    let mobile = MobileService::new(
        pool.clone(),
        auth.clone(),
        CoreRpc::unreachable(),
        &soundcloud,
    );
    let session_id = signed_in(pool).await?;
    mobile_app(pool).await?;
    Ok(Fixture {
        mobile,
        auth,
        session_id,
        exchanges,
    })
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

async fn signed_in(pool: &PgPool) -> anyhow::Result<Uuid> {
    let web_app = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ($1, 'web', 'fake-web-client', 'fake-web-secret', 'https://app.test/')",
    )
    .bind(web_app)
    .execute(pool)
    .await?;
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, oauth_app_id, access_token, refresh_token,
             expires_at, scope)
         VALUES ($1, $2, 'fake-listener', $3, 'web-access', 'web-refresh',
                 now() + interval '1 hour', '*')",
    )
    .bind(connection)
    .bind(ACCOUNT)
    .bind(web_app)
    .execute(pool)
    .await?;
    let session = Uuid::now_v7();
    sqlx::query("INSERT INTO sessions (id, soundcloud_connection_id) VALUES ($1, $2)")
        .bind(session)
        .bind(connection)
        .execute(pool)
        .await?;
    Ok(session)
}

async fn mobile_app(pool: &PgPool) -> anyhow::Result<Uuid> {
    let app = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (
             id, name, client_id, client_secret, redirect_uri,
             client_kind, app_id, app_version, user_agent
         ) VALUES (
             $1, 'mobile', 'fake-mobile-client', 'fake-mobile-secret', 'sc://auth',
             'mobile', 'fake.app', '1.0.0', 'FakeAgent/1.0'
         )",
    )
    .bind(app)
    .execute(pool)
    .await?;
    Ok(app)
}

async fn started(fixture: &Fixture) -> anyhow::Result<(Uuid, Secret<String>)> {
    let started = fixture.mobile.start(fixture.session_id, ACCOUNT).await?;
    let url = Url::parse(&started.url)?;
    let state = url
        .query_pairs()
        .find_map(|(name, value)| (name == "state").then(|| value.into_owned()))
        .ok_or_else(|| anyhow::anyhow!("the authorize URL carries no state"))?;
    assert_eq!(started.account.username, "fake-listener");
    Ok((started.id, Secret::new(state)))
}

async fn callback(
    fixture: &Fixture,
    id: Uuid,
    state: &Secret<String>,
) -> Result<LinkStatus, AppError> {
    fixture
        .mobile
        .callback(
            fixture.session_id,
            ACCOUNT,
            id,
            &Secret::new("fake-code".to_owned()),
            state,
        )
        .await
}

async fn credentials(pool: &PgPool) -> anyhow::Result<Vec<(String, String, Option<String>)>> {
    Ok(sqlx::query_as(
        "SELECT soundcloud_user_id, access_token, plan FROM mobile_credentials ORDER BY 1",
    )
    .fetch_all(pool)
    .await?)
}

fn completed() -> LinkStatus {
    LinkStatus {
        status: "completed".to_owned(),
        error: None,
    }
}

pub(super) fn status_of(error: AppError) -> StatusCode {
    error.into_response().status()
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_same_account_links_and_keeps_its_tokens_and_plan(pool: PgPool) -> anyhow::Result<()> {
    let fixture = fixture(&pool, Token::Pair, Me::Account(ACCOUNT)).await?;
    let (id, state) = started(&fixture).await?;

    let linked = callback(&fixture, id, &state).await?;

    assert_eq!(linked, completed());
    assert_eq!(
        credentials(&pool).await?,
        vec![(
            ACCOUNT.to_owned(),
            "fake-mobile-access-1".to_owned(),
            Some("none".to_owned())
        )]
    );
    assert_eq!(
        fixture.mobile.status(fixture.session_id, id).await?,
        completed()
    );
    let overview = fixture.mobile.overview(fixture.session_id, ACCOUNT).await?;
    assert!(overview.linked && overview.available && !overview.uq_opt_in);
    assert_eq!(overview.status.as_deref(), Some("active"));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn another_account_writes_nothing_and_an_old_link_survives(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = fixture(&pool, Token::Pair, Me::Account(OTHER_ACCOUNT)).await?;
    let (id, state) = started(&fixture).await?;

    let refused = callback(&fixture, id, &state).await?;

    assert_eq!(refused.status, "account_mismatch");
    assert!(credentials(&pool).await?.is_empty());

    old_link(&pool).await?;
    let (id, state) = started(&fixture).await?;
    let refused = callback(&fixture, id, &state).await?;

    assert_eq!(refused.status, "account_mismatch");
    assert_eq!(
        credentials(&pool).await?,
        vec![(ACCOUNT.to_owned(), "old-mobile-access".to_owned(), None)]
    );
    Ok(())
}

pub(super) async fn old_link(pool: &PgPool) -> anyhow::Result<()> {
    let app: Uuid = sqlx::query_scalar("SELECT id FROM oauth_apps WHERE client_kind = 'mobile'")
        .fetch_one(pool)
        .await?;
    sqlx::query(
        "INSERT INTO mobile_credentials (
             soundcloud_user_id, oauth_app_id, device_id, access_token, refresh_token,
             expires_at, scope, linked_at
         ) VALUES ($1, $2, $3, 'old-mobile-access', 'old-mobile-refresh',
                   now() + interval '6 days', '*', now())",
    )
    .bind(ACCOUNT)
    .bind(app)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_repeated_callback_is_answered_without_a_second_exchange(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = fixture(&pool, Token::Pair, Me::Account(ACCOUNT)).await?;
    let (id, state) = started(&fixture).await?;

    let first = callback(&fixture, id, &state).await?;
    let second = callback(&fixture, id, &state).await?;

    assert_eq!(first, completed());
    assert_eq!(second, completed());
    assert_eq!(fixture.exchanges.load(Ordering::SeqCst), 1);
    assert_eq!(credentials(&pool).await?.len(), 1);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_relink_replaces_the_tokens_of_the_same_row(pool: PgPool) -> anyhow::Result<()> {
    let fixture = fixture(&pool, Token::Pair, Me::Account(ACCOUNT)).await?;
    old_link(&pool).await?;
    let (id, state) = started(&fixture).await?;

    let linked = callback(&fixture, id, &state).await?;

    assert_eq!(linked, completed());
    assert_eq!(
        credentials(&pool).await?,
        vec![(
            ACCOUNT.to_owned(),
            "fake-mobile-access-1".to_owned(),
            Some("none".to_owned())
        )]
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_foreign_id_a_wrong_state_and_an_expired_request_are_refused(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = fixture(&pool, Token::Pair, Me::Account(ACCOUNT)).await?;
    let (id, state) = started(&fixture).await?;

    let foreign = callback(&fixture, Uuid::now_v7(), &state).await;
    let wrong = callback(&fixture, id, &Secret::new("forged".to_owned())).await;
    sqlx::query("UPDATE mobile_link_requests SET expires_at = now() - interval '1 second'")
        .execute(&pool)
        .await?;
    let expired = callback(&fixture, id, &state).await;
    let polled = fixture.mobile.status(fixture.session_id, id).await?;

    assert_eq!(foreign.map_err(status_of), Err(StatusCode::NOT_FOUND));
    assert_eq!(wrong.map_err(status_of), Err(StatusCode::BAD_REQUEST));
    assert_eq!(expired.map_err(status_of), Err(StatusCode::GONE));
    assert_eq!(polled.status, "expired");
    assert_eq!(fixture.exchanges.load(Ordering::SeqCst), 0);
    assert!(credentials(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_rejected_profile_right_after_the_exchange_fails_without_a_mismatch(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = fixture(&pool, Token::Pair, Me::Rejected).await?;
    let (id, state) = started(&fixture).await?;

    let failed = callback(&fixture, id, &state).await?;

    assert_eq!(
        failed,
        LinkStatus {
            status: "failed".to_owned(),
            error: Some("profile_rejected".to_owned()),
        }
    );
    assert!(credentials(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_captcha_on_the_exchange_is_a_plain_refusal(pool: PgPool) -> anyhow::Result<()> {
    let fixture = fixture(&pool, Token::Captcha, Me::Account(ACCOUNT)).await?;
    let (id, state) = started(&fixture).await?;

    let failed = callback(&fixture, id, &state).await?;
    let again = callback(&fixture, id, &state).await?;

    assert_eq!(failed.error.as_deref(), Some("captcha"));
    assert_eq!(again, failed);
    assert_eq!(fixture.exchanges.load(Ordering::SeqCst), 1);
    assert!(credentials(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_unlink_drops_the_profile_and_keeps_the_session(pool: PgPool) -> anyhow::Result<()> {
    let fixture = fixture(&pool, Token::Pair, Me::Account(ACCOUNT)).await?;
    let (id, state) = started(&fixture).await?;
    callback(&fixture, id, &state).await?;

    fixture.mobile.unlink(ACCOUNT).await?;

    assert!(credentials(&pool).await?.is_empty());
    assert!(
        fixture
            .auth
            .find_session(fixture.session_id)
            .await?
            .is_some()
    );
    assert_eq!(
        fixture
            .auth
            .get_valid_access_token(fixture.session_id)
            .await?,
        "web-access"
    );
    let overview = fixture.mobile.overview(fixture.session_id, ACCOUNT).await?;
    assert!(!overview.linked);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_last_session_of_the_account_takes_the_mobile_link_with_it(
    pool: PgPool,
) -> anyhow::Result<()> {
    let fixture = fixture(&pool, Token::Pair, Me::Account(ACCOUNT)).await?;
    let (id, state) = started(&fixture).await?;
    callback(&fixture, id, &state).await?;

    fixture.auth.logout(fixture.session_id).await?;

    assert!(credentials(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_web_connection_never_sees_the_mobile_profile(pool: PgPool) -> anyhow::Result<()> {
    let fixture = fixture(&pool, Token::Pair, Me::Account(ACCOUNT)).await?;
    let (id, state) = started(&fixture).await?;
    callback(&fixture, id, &state).await?;

    let token = fixture
        .auth
        .get_valid_access_token(fixture.session_id)
        .await?;
    let connections: i64 = sqlx::query_scalar("SELECT count(*) FROM soundcloud_connections")
        .fetch_one(&pool)
        .await?;

    assert_eq!(token, "web-access");
    assert_eq!(connections, 1);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_account_outside_the_allow_list_cannot_start(pool: PgPool) -> anyhow::Result<()> {
    let fixture = fixture(&pool, Token::Pair, Me::Account(ACCOUNT)).await?;
    let closed = MobileService::new(
        pool.clone(),
        fixture.auth.clone(),
        CoreRpc::unreachable(),
        &SoundcloudCfg {
            proxy_url: String::new(),
            proxy_fallback: false,
            mobile_link: MobileLinkAllow::parse(OTHER_ACCOUNT),
        },
    );

    let refused = closed.start(fixture.session_id, ACCOUNT).await;
    let overview = closed.overview(fixture.session_id, ACCOUNT).await?;

    assert_eq!(
        refused.map(|_| ()).map_err(status_of),
        Err(StatusCode::FORBIDDEN)
    );
    assert!(!overview.available && !overview.linked);
    Ok(())
}
