use std::sync::Arc;

use axum::extract::State;
use chrono::Utc;
use sc_chat_wire::bind::make_bind;
use sc_chat_wire::prost::Message;
use sc_chat_wire::proto::Inner;
use sc_chat_wire::random_bytes;
use sc_chat_wire::sign::sign_request;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use crate::modules::auth::model::LoginRequest;
use crate::modules::auth::service::login::LoginIdentity;
use crate::modules::chat::body::ChatJson;
use crate::modules::chat::caller::{CallerBind, resolve};
use crate::modules::chat::devices::{self, RegisterDevice};
use crate::modules::chat::sweeps::devices::Revoker;
use crate::modules::chat::test_support::chat_state;
use crate::modules::chat::{ChatError, ChatResult, ChatState, requests};
use crate::modules::rooms::identity::account_profile;
use crate::modules::rooms::model::Profile;
use crate::sc::ScTokenResponse;

const FUNCTIONS: &str = "public.auth_session(uuid), public.auth_sessions_alive(uuid[]), \
    public.auth_session_username(uuid), public.auth_session_end(uuid), \
    public.auth_login_create(text, text, uuid, bytea), public.auth_login_status(uuid), \
    public.auth_link_create(text, text, uuid, bytea), \
    public.auth_link_claim(text, uuid, bytea, bytea), public.auth_link_status(uuid)";

struct Restricted {
    role: String,
    pool: PgPool,
}

impl Restricted {
    async fn new(owner: &PgPool) -> anyhow::Result<Self> {
        let role = format!("chat_{}", Uuid::new_v4().simple());
        let password = Uuid::new_v4().simple().to_string();
        sqlx::raw_sql(&format!(
            "CREATE ROLE {role} LOGIN PASSWORD '{password}'
                 NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS NOREPLICATION;
             GRANT USAGE ON SCHEMA chat TO {role};
             GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA chat TO {role};
             GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA chat TO {role};
             GRANT SELECT ON public._sqlx_migrations, public.users, public.user_followings,
                 public.user_blocked_artists, public.subscriptions TO {role};
             REVOKE ALL ON public.sessions, public.login_requests, public.link_requests,
                 public.soundcloud_connections, public.repl_tombstones,
                 public.sync_queue_action_pauses FROM {role};
             REVOKE ALL ON SEQUENCE public.scd_orphan_sessions FROM {role};
             GRANT EXECUTE ON FUNCTION {FUNCTIONS} TO {role};"
        ))
        .execute(owner)
        .await?;
        let options = owner
            .connect_options()
            .as_ref()
            .clone()
            .username(&role)
            .password(&password);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        Ok(Self { role, pool })
    }

    async fn remove(self, owner: &PgPool) -> anyhow::Result<()> {
        self.pool.close().await;
        let role = self.role;
        sqlx::raw_sql(&format!("DROP OWNED BY {role}; DROP ROLE {role};"))
            .execute(owner)
            .await?;
        Ok(())
    }
}

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

async fn connected(owner: &PgPool, user: &str, bind: Option<&[u8]>) -> anyhow::Result<Uuid> {
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

async fn denied(pool: &PgPool, sql: &str) {
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

    chat.auth.logout(session).await?;
    assert_eq!(session_bind(owner, session).await?, None);
    Ok(())
}

async fn sweep(owner: &PgPool, chat: &ChatState) -> anyhow::Result<()> {
    let user = "7201";
    sqlx::query(
        "WITH live AS (
             INSERT INTO sessions (id) SELECT gen_random_uuid() FROM generate_series(1, 501)
             RETURNING id
         )
         INSERT INTO chat.devices
             (id, user_id, session_id, auth_pub, box_pub, platform, name, app_version, tier,
              last_seen_at)
         SELECT gen_random_uuid(), $1, live.id, '\\x01'::bytea, '\\x02'::bytea, 'linux', 'old',
                '8.6.0', 'direct', now() - interval '3 hours'
         FROM live",
    )
    .bind(user)
    .execute(owner)
    .await?;
    let gone = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO chat.devices
             (id, user_id, session_id, auth_pub, box_pub, platform, name, app_version, tier,
              last_seen_at)
         VALUES ($1, $2, $3, '\\x01'::bytea, '\\x02'::bytea, 'linux', 'gone', '8.6.0', 'direct',
                 now() - interval '2 hours')",
    )
    .bind(gone)
    .bind(user)
    .bind(Uuid::now_v7())
    .execute(owner)
    .await?;

    let mut revoker = Revoker::default();
    assert_eq!(ok(revoker.run_once(chat).await)?, 1);
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
    assert_eq!(active, 501);
    assert_eq!(ok(revoker.run_once(chat).await)?, 0);
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

async fn bound_login(owner: &PgPool, chat: &ChatState) -> anyhow::Result<()> {
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
    assert!(started.url.contains(&format!("client_id=client-{app}")));
    assert!(!started.url.contains("never-leaves"));
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

    drop(chat);
    restricted.remove(&owner).await
}
