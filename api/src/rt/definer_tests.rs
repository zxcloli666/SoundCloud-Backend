use std::sync::Arc;

use axum::extract::State;
use axum::http::header::RETRY_AFTER;
use axum::response::IntoResponse;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use sc_chat_wire::bind::make_bind;
use sc_chat_wire::prost::Message;
use sc_chat_wire::proto::Inner;
use sc_chat_wire::random_bytes;
use sc_chat_wire::sign::sign_request;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::{Uuid, Version};

use crate::common::session::session_hash;
use crate::error::AppError;
use crate::modules::auth::definers::CHAT_FUNCTIONS;
use crate::modules::auth::health::oauth_app_key;
use crate::modules::auth::model::LoginRequest;
use crate::modules::auth::service::login::LoginIdentity;
use crate::modules::auth::start_limit::STARTS_LIMITED;
use crate::modules::auth::test_support::Restricted;
use crate::modules::chat::body::ChatJson;
use crate::modules::chat::caller::{CallerBind, resolve};
use crate::modules::chat::devices::{self, RegisterDevice};
use crate::modules::chat::sweeps::devices::{self as device_sweep, BATCH_ROWS};
use crate::modules::chat::test_support::chat_state;
use crate::modules::chat::{ChatError, ChatResult, ChatState, requests};
use crate::modules::rooms::identity::account_profile;
use crate::modules::rooms::model::Profile;
use crate::sc::ScTokenResponse;

struct Device {
    id: Uuid,
    seed: [u8; 32],
    bind: Vec<u8>,
}

impl Device {
    fn new() -> Self {
        let id = Uuid::new_v4();
        let seed = random();
        let now = u64::try_from(Utc::now().timestamp_millis()).expect("a positive clock");
        let bind = make_bind(&id.to_string(), &seed, &random(), now).encode_to_vec();
        Self { id, seed, bind }
    }

    fn owns(&self) -> impl Fn(&[u8]) -> bool {
        let keys = CallerBind::decode(&self.bind).expect("bind keys");
        move |stored| keys.owns(stored)
    }

    fn signed(&self, session: Uuid, enc: &[u8; 32]) -> Inner {
        let mut inner = Inner {
            ts_ms: u64::try_from(Utc::now().timestamp_millis()).expect("a positive clock"),
            session_id: Some(session.to_string()),
            device_id: Some(self.id.to_string()),
            method: "POST".to_owned(),
            path: "/chat/devices".to_owned(),
            ..Inner::default()
        };
        sign_request(&self.seed, &mut inner, enc);
        inner
    }
}

fn ok<T>(result: ChatResult<T>) -> anyhow::Result<T> {
    result.map_err(|error| anyhow::anyhow!("{error:?}"))
}

fn random() -> [u8; 32] {
    random_bytes().expect("randomness")
}

fn redis() -> deadpool_redis::Pool {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    crate::redis::connect_url(&url).expect("a redis pool")
}

pub(super) async fn connected(
    owner: &PgPool,
    user: &str,
    bind: Option<&[u8]>,
) -> anyhow::Result<Uuid> {
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, $2, $2, 'access', 'refresh', now() + interval '1 hour', '')",
    )
    .bind(connection)
    .bind(user)
    .execute(owner)
    .await?;
    let session = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO sessions (id, soundcloud_connection_id, chat_bind) VALUES ($1, $2, $3)",
    )
    .bind(session)
    .bind(connection)
    .bind(bind)
    .execute(owner)
    .await?;
    Ok(session)
}

async fn session_bind(owner: &PgPool, session: Uuid) -> anyhow::Result<Option<Option<Vec<u8>>>> {
    Ok(
        sqlx::query_scalar("SELECT chat_bind FROM sessions WHERE id = $1")
            .bind(session)
            .fetch_optional(owner)
            .await?,
    )
}

pub(super) async fn denied(pool: &PgPool, sql: &str) {
    let error = sqlx::raw_sql(sql)
        .execute(pool)
        .await
        .expect_err("the chat role must not reach this");
    let code = error
        .as_database_error()
        .and_then(|error| error.code())
        .map(|code| code.into_owned());
    assert_eq!(code.as_deref(), Some("42501"), "{sql}: {error}");
}

async fn caller_and_device(owner: &PgPool, chat: &Arc<ChatState>) -> anyhow::Result<()> {
    let device = Device::new();
    let session = connected(owner, "7101", Some(&device.bind)).await?;
    let enc = random();

    let resolved = ok(resolve(chat, &device.signed(session, &enc), &enc).await)?;
    assert_eq!(resolved.caller.user_id, "7101");
    assert_eq!(resolved.bind.map(|bind| bind.device_id), Some(device.id));

    let unknown = resolve(chat, &device.signed(Uuid::now_v7(), &enc), &enc)
        .await
        .err();
    assert_eq!(unknown, Some(ChatError::ReloginRequired));

    let registered = ok(devices::register(
        resolved.caller.clone(),
        State(chat.clone()),
        ChatJson(RegisterDevice {
            platform: "linux".to_owned(),
            name: "test".to_owned(),
            app_version: "8.6.0".to_owned(),
            caps: Vec::new(),
        }),
    )
    .await)?;
    assert_eq!(registered.0.id, device.id);

    assert!(ok(requests::session_is_young(chat, session).await)?);
    assert!(!ok(requests::session_is_young(chat, Uuid::now_v7()).await)?);

    let found = chat.auth.find_session_cached(session).await?;
    assert_eq!(
        found.and_then(|row| row.soundcloud_user_id),
        Some("7101".to_owned())
    );
    let claimed = Profile {
        name: "claimed".to_owned(),
        avatar_url: None,
    };
    let profile = account_profile(&chat.pg, "7101", session, claimed).await?;
    assert_eq!(profile.name, "7101");

    let stranger = connected(owner, "7102", None).await?;
    assert!(
        !chat
            .auth
            .end_session(stranger, &session_hash(session))
            .await?
    );
    assert!(session_bind(owner, session).await?.is_some());

    chat.auth.logout(session).await?;
    assert_eq!(session_bind(owner, session).await?, None);
    Ok(())
}

async fn sweep(owner: &PgPool, chat: &ChatState) -> anyhow::Result<()> {
    let user = "7201";
    sqlx::query(
        "WITH live AS (
             INSERT INTO sessions (id) SELECT gen_random_uuid() FROM generate_series(1, $2)
             RETURNING id
         )
         INSERT INTO chat.devices
             (id, user_id, session_hash, auth_pub, box_pub, platform, name, app_version, tier,
              last_seen_at)
         SELECT gen_random_uuid(), $1, sha256(uuid_send(live.id)), '\\x01'::bytea, '\\x02'::bytea, 'linux', 'old',
                '8.6.0', 'direct', now() - interval '3 hours'
         FROM live",
    )
    .bind(user)
    .bind(BATCH_ROWS + 1)
    .execute(owner)
    .await?;
    let gone = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chat.devices
             (id, user_id, session_hash, auth_pub, box_pub, platform, name, app_version, tier,
              last_seen_at)
         VALUES ($1, $2, sha256(uuid_send($3)), '\\x01'::bytea, '\\x02'::bytea, 'linux', 'gone', '8.6.0', 'direct',
                 now() - interval '2 hours')",
    )
    .bind(gone)
    .bind(user)
    .bind(Uuid::now_v7())
    .execute(owner)
    .await?;

    assert_eq!(ok(device_sweep::run_once(chat).await)?, 1);
    let status: String = sqlx::query_scalar("SELECT status FROM chat.devices WHERE id = $1")
        .bind(gone)
        .fetch_one(owner)
        .await?;
    assert_eq!(status, "revoked");
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chat.devices WHERE user_id = $1 AND status = 'active'",
    )
    .bind(user)
    .fetch_one(owner)
    .await?;
    assert_eq!(active, BATCH_ROWS + 1);
    assert_eq!(ok(device_sweep::run_once(chat).await)?, 0);
    Ok(())
}

async fn finish_login(owner: &PgPool, login_request_id: Uuid, user: &str) -> anyhow::Result<Uuid> {
    let state: String = sqlx::query_scalar("SELECT state FROM login_requests WHERE id = $1")
        .bind(login_request_id)
        .fetch_one(owner)
        .await?;
    let claimed = sqlx::query_file_as!(
        LoginRequest,
        "queries/auth/service/claim_login_request.sql",
        state
    )
    .fetch_one(owner)
    .await?;
    let token = ScTokenResponse {
        access_token: format!("access-{user}"),
        refresh_token: format!("refresh-{user}"),
        expires_in: 3_600,
        scope: String::new(),
        token_type: "bearer".to_owned(),
    };
    let oauth_app_id: Uuid = claimed
        .oauth_app_id
        .as_deref()
        .unwrap_or_default()
        .parse()?;
    let owner_chat = chat_state(owner.clone(), redis(), true);
    Ok(owner_chat
        .auth
        .finish_login(
            &claimed,
            LoginIdentity {
                soundcloud_user_id: user,
                username: Some(user),
                profile: None,
                oauth_app_id,
                token: &token,
                expires_at: Utc::now() + chrono::Duration::hours(1),
                observation: catalog_ingest::Observation::begin(owner).await?,
            },
        )
        .await?)
}

async fn sick_app(owner: &PgPool) -> anyhow::Result<String> {
    let sick = Uuid::now_v7();
    let client_id = format!("client-{sick}");
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri, created_at)
         VALUES ($1, 'sick', $2, 'never-leaves', 'https://app.test/', now() - interval '1 hour')",
    )
    .bind(sick)
    .bind(&client_id)
    .execute(owner)
    .await?;
    let key = oauth_app_key(&client_id);
    let mut conn = redis().get().await?;
    let _: () = redis::cmd("SET")
        .arg(format!("auth:app:{key}:failure"))
        .arg(10)
        .arg("EX")
        .arg(300)
        .query_async(&mut conn)
        .await?;
    Ok(key)
}

async fn bound_login(owner: &PgPool, chat: &ChatState) -> anyhow::Result<()> {
    let sick = sick_app(owner).await?;
    let app = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ($1, 'test', $2, 'never-leaves', 'https://app.test/')",
    )
    .bind(app)
    .bind(format!("client-{app}"))
    .execute(owner)
    .await?;
    let device = Device::new();
    let stranger = Device::new();

    let started = chat.auth.initiate_bound_login(None, &device.bind).await?;
    let mut conn = redis().get().await?;
    let _: () = redis::cmd("DEL")
        .arg(format!("auth:app:{sick}:failure"))
        .query_async(&mut conn)
        .await?;
    assert!(
        started.url.contains(&format!("client_id=client-{app}")),
        "a login avoids the app the health counters mark as failing"
    );
    assert!(!started.url.contains("never-leaves"));
    assert_eq!(
        started.login_request_id.get_version(),
        Some(Version::SortRand)
    );
    let (state, verifier): (String, String) =
        sqlx::query_as("SELECT state, code_verifier FROM login_requests WHERE id = $1")
            .bind(started.login_request_id)
            .fetch_one(owner)
            .await?;
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    assert_eq!((state.len(), verifier.len()), (32, 43));
    assert!(started.url.contains(&format!("state={state}")));
    assert!(started.url.contains(&format!("code_challenge={challenge}")));
    assert!(!started.url.contains(&verifier));
    let pending = chat
        .auth
        .login_request_status_for(started.login_request_id, device.owns())
        .await?;
    assert_eq!(pending.status, "pending");

    let session = finish_login(owner, started.login_request_id, "7301").await?;
    let done = chat
        .auth
        .login_request_status_for(started.login_request_id, device.owns())
        .await?;
    assert_eq!(done.status, "completed");
    assert_eq!(done.session_id, Some(session));
    let foreign = chat
        .auth
        .login_request_status_for(started.login_request_id, stranger.owns())
        .await?;
    assert_eq!(foreign.session_id, None);
    assert_eq!(session_bind(owner, session).await?, Some(Some(device.bind)));
    Ok(())
}

async fn links(owner: &PgPool, chat: &ChatState) -> anyhow::Result<()> {
    let source = Device::new();
    let source_session = connected(owner, "7401", Some(&source.bind)).await?;

    let asking = Device::new();
    let pull = chat
        .link
        .create_bound("pull", None, Some(&asking.bind), asking.owns())
        .await?;
    let claimed = chat
        .link
        .claim_bound(
            &pull.claim_token,
            Some(source_session),
            Some(&source.bind),
            source.owns(),
        )
        .await?;
    assert_eq!(claimed.mode, "pull");
    for minted in [pull.link_request_id, claimed.session_id] {
        assert_eq!(minted.get_version(), Some(Version::SortRand), "{minted}");
    }
    assert_eq!(
        session_bind(owner, claimed.session_id).await?,
        Some(Some(asking.bind.clone()))
    );
    let polled = chat
        .link
        .get_status_for(pull.link_request_id, asking.owns())
        .await?;
    assert_eq!(polled.status, "claimed");
    assert_eq!(polled.session_id, Some(claimed.session_id));
    let peeked = chat
        .link
        .get_status_for(pull.link_request_id, source.owns())
        .await?;
    assert_eq!(peeked.session_id, None);

    let racing = chat
        .link
        .create_bound("pull", None, Some(&asking.bind), asking.owns())
        .await?;
    let (first, second) = tokio::join!(
        chat.link.claim_bound(
            &racing.claim_token,
            Some(source_session),
            Some(&source.bind),
            source.owns(),
        ),
        chat.link.claim_bound(
            &racing.claim_token,
            Some(source_session),
            Some(&source.bind),
            source.owns(),
        )
    );
    assert_eq!(
        [first.is_ok(), second.is_ok()]
            .into_iter()
            .filter(|claimed| *claimed)
            .count(),
        1,
        "a claim token works once even when two claims race"
    );

    let joining = Device::new();
    let push = chat
        .link
        .create_bound(
            "push",
            Some(source_session),
            Some(&source.bind),
            source.owns(),
        )
        .await?;
    let pushed = chat
        .link
        .claim_bound(&push.claim_token, None, Some(&joining.bind), joining.owns())
        .await?;
    assert_eq!(
        session_bind(owner, pushed.session_id).await?,
        Some(Some(joining.bind.clone()))
    );
    let again = chat
        .link
        .claim_bound(&push.claim_token, None, Some(&joining.bind), joining.owns())
        .await;
    assert!(again.is_err(), "a claim token works once");

    let late = chat
        .link
        .create_bound("pull", None, Some(&asking.bind), asking.owns())
        .await?;
    sqlx::query(
        "UPDATE link_requests SET expires_at = expires_at - interval '1 hour' WHERE id = $1",
    )
    .bind(late.link_request_id)
    .execute(owner)
    .await?;
    let expired = chat
        .link
        .claim_bound(
            &late.claim_token,
            Some(source_session),
            Some(&source.bind),
            source.owns(),
        )
        .await;
    assert!(expired.is_err());
    let marked: String = sqlx::query_scalar("SELECT status FROM link_requests WHERE id = $1")
        .bind(late.link_request_id)
        .fetch_one(owner)
        .await?;
    assert_eq!(marked, "failed");
    Ok(())
}

const STARTS_PER_MINUTE: i64 = 3_000;

async fn fill_window(owner: &PgPool, kind: &str, age: &str) -> anyhow::Result<()> {
    sqlx::query(&format!(
        "INSERT INTO auth_caps.starts (kind, at)
         SELECT $1, clock_timestamp() - interval '{age}' FROM generate_series(1, $2)"
    ))
    .bind(kind)
    .bind(STARTS_PER_MINUTE)
    .execute(owner)
    .await?;
    Ok(())
}

fn refusal<T>(result: Result<T, AppError>) -> Option<(&'static str, Option<i64>)> {
    let error = result.err()?;
    let code = error.public_code();
    let retry_after = error
        .into_response()
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok());
    Some((code, retry_after))
}

async fn window(owner: &PgPool, kind: &str) -> anyhow::Result<i64> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM auth_caps.starts WHERE kind = $1")
            .bind(kind)
            .fetch_one(owner)
            .await?,
    )
}

async fn starts_capped(owner: &PgPool, chat: &ChatState) -> anyhow::Result<()> {
    sqlx::raw_sql("DELETE FROM auth_caps.starts")
        .execute(owner)
        .await?;
    let device = Device::new();
    fill_window(owner, "login", "61 seconds").await?;
    chat.auth.initiate_bound_login(None, &device.bind).await?;
    assert_eq!(
        window(owner, "login").await?,
        1,
        "starts older than a minute fall out"
    );
    sqlx::raw_sql("DELETE FROM auth_caps.starts")
        .execute(owner)
        .await?;
    fill_window(owner, "login", "30 seconds").await?;
    let login = refusal(chat.auth.initiate_bound_login(None, &device.bind).await);
    assert!(
        matches!(login, Some((STARTS_LIMITED, Some(29..=31)))),
        "the refusal waits for the oldest start to age out: {login:?}"
    );

    fill_window(owner, "link", "0 seconds").await?;
    let link = refusal(
        chat.link
            .create_bound("pull", None, Some(&device.bind), device.owns())
            .await,
    );
    assert!(
        matches!(link, Some((STARTS_LIMITED, Some(59..=60)))),
        "{link:?}"
    );

    let logins = window(owner, "login").await?;
    let api = chat_state(owner.clone(), redis(), true);
    api.link.create("pull", None).await?;
    api.auth.initiate_bound_login(None, &device.bind).await?;
    assert_eq!(
        (window(owner, "link").await?, window(owner, "login").await?),
        (STARTS_PER_MINUTE, logins),
        "the main API is neither capped nor counted"
    );

    let mut repeatable = chat.pg.begin().await?;
    sqlx::raw_sql("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *repeatable)
        .await?;
    let stale = sqlx::query("SELECT * FROM public.auth_link_create('pull', NULL, '\\x01')")
        .execute(&mut *repeatable)
        .await
        .expect_err("a stale snapshot cannot count starts");
    assert_eq!(
        stale
            .as_database_error()
            .and_then(|error| error.code())
            .as_deref(),
        Some("25000")
    );
    repeatable.rollback().await?;
    Ok(())
}

async fn walls(pool: &PgPool) {
    denied(pool, "SELECT id FROM public.sessions LIMIT 1").await;
    denied(
        pool,
        "SELECT result_session_id FROM public.login_requests LIMIT 1",
    )
    .await;
    denied(pool, "SELECT claim_token FROM public.link_requests LIMIT 1").await;
    denied(
        pool,
        "SELECT access_token FROM public.soundcloud_connections LIMIT 1",
    )
    .await;
    denied(pool, "SELECT client_secret FROM public.oauth_apps LIMIT 1").await;
    denied(
        pool,
        "INSERT INTO public.sessions (id) VALUES (gen_random_uuid())",
    )
    .await;
    denied(pool, "SELECT nextval('public.scd_orphan_sessions')").await;
    denied(pool, "CREATE TABLE public.chat_probe (id int)").await;
    denied(pool, "SELECT public.auth_pin_scd()").await;
    denied(pool, "SELECT public.auth_secret(24)").await;
    denied(
        pool,
        "SELECT public.auth_connection_release(gen_random_uuid())",
    )
    .await;
    denied(pool, "SELECT public.auth_start_window('login', 1)").await;
    denied(pool, "SELECT public.auth_uuid_v7()").await;
    denied(pool, "SELECT count(*) FROM auth_caps.starts").await;
    for function in CHAT_FUNCTIONS {
        let open: bool =
            sqlx::query_scalar("SELECT has_function_privilege('public', $1, 'EXECUTE')")
                .bind(function)
                .fetch_one(pool)
                .await
                .expect("a privilege check");
        assert!(!open, "{function} must not be executable by PUBLIC");
    }
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn the_chat_role_runs_every_auth_flow_through_definer_functions(
    owner: PgPool,
) -> anyhow::Result<()> {
    let restricted = Restricted::new(&owner).await?;
    let chat = chat_state(restricted.pool.clone(), redis(), true);

    walls(&restricted.pool).await;
    caller_and_device(&owner, &chat).await?;
    sweep(&owner, &chat).await?;
    bound_login(&owner, &chat).await?;
    links(&owner, &chat).await?;
    starts_capped(&owner, &chat).await?;

    drop(chat);
    restricted.remove(&owner).await
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_chat_role_on_a_split_node_asks_for_a_retry_on_a_young_miss(
    owner: PgPool,
) -> anyhow::Result<()> {
    sqlx::query(
        "DO $$ BEGIN EXECUTE format('ALTER DATABASE %I SET scd.mode = %L', current_database(), 'split'); END $$",
    )
    .execute(&owner)
    .await?;
    let restricted = Restricted::new(&owner).await?;
    let chat = chat_state(
        restricted.pool.clone(),
        crate::redis::connect_url("redis://127.0.0.1:1")?,
        true,
    );
    let device = Device::new();
    let enc = random();

    let missed = resolve(&chat, &device.signed(Uuid::now_v7(), &enc), &enc)
        .await
        .err();
    denied(
        &restricted.pool,
        "SELECT subconninfo FROM pg_catalog.pg_subscription",
    )
    .await;
    denied(
        &restricted.pool,
        "SELECT 1 FROM public.repl_tombstones LIMIT 1",
    )
    .await;

    drop(chat);
    restricted.remove(&owner).await?;
    assert_eq!(missed, Some(ChatError::SessionNotReplicated));
    Ok(())
}

async fn refused_with(pool: &PgPool, sql: &str, count: i32) -> Option<String> {
    sqlx::query(sql)
        .bind(count)
        .execute(pool)
        .await
        .err()
        .and_then(|error| {
            error
                .as_database_error()
                .and_then(|error| error.code())
                .map(|code| code.into_owned())
        })
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_auth_functions_cap_the_arrays_they_take(owner: PgPool) -> anyhow::Result<()> {
    let restricted = Restricted::new(&owner).await?;
    let login = "SELECT * FROM public.auth_login_create(NULL, '\\x01',
                     ARRAY(SELECT gen_random_uuid() FROM generate_series(1, $1)))";
    let alive = "SELECT * FROM public.auth_sessions_alive(
                     ARRAY(SELECT sha256(uuid_send(gen_random_uuid())) FROM generate_series(1, $1)))";

    let outcomes = [
        refused_with(&restricted.pool, login, 100).await,
        refused_with(&restricted.pool, login, 101).await,
        refused_with(&restricted.pool, alive, 1000).await,
        refused_with(&restricted.pool, alive, 1001).await,
    ];
    restricted.remove(&owner).await?;

    assert_eq!(
        outcomes,
        [
            None,
            Some("54000".to_owned()),
            None,
            Some("54000".to_owned())
        ]
    );
    Ok(())
}
