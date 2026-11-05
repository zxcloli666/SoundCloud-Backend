use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use chrono::Utc;
use sc_chat_wire::bind::make_bind;
use sc_chat_wire::prost::Message;
use sc_chat_wire::proto::Inner;
use sc_chat_wire::random_bytes;
use sc_chat_wire::sign::sign_request;
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::session::session_hash;
use crate::modules::auth::definers::CHAT_FUNCTIONS;
use crate::modules::auth::test_support::{Restricted, SEALED_FUNCTIONS};
use crate::modules::chat::body::ChatJson;
use crate::modules::chat::caller::resolve;
use crate::modules::chat::devices::{self, RegisterDevice};
use crate::modules::chat::sweeps::devices::{self as device_sweep, BATCH_ROWS};
use crate::modules::chat::test_support::chat_state;
use crate::modules::chat::{ChatError, ChatResult, ChatState, requests};
use crate::modules::rooms::identity::account_profile;
use crate::modules::rooms::model::Profile;

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
    for function in CHAT_FUNCTIONS.into_iter().chain(SEALED_FUNCTIONS) {
        let open: bool =
            sqlx::query_scalar("SELECT has_function_privilege('public', $1, 'EXECUTE')")
                .bind(function)
                .fetch_one(pool)
                .await
                .expect("a privilege check");
        assert!(!open, "{function} must not be executable by PUBLIC");
    }
    for function in SEALED_FUNCTIONS {
        let held: bool =
            sqlx::query_scalar("SELECT has_function_privilege(current_user, $1, 'EXECUTE')")
                .bind(function)
                .fetch_one(pool)
                .await
                .expect("a privilege check");
        assert!(!held, "{function} must not be executable by the chat role");
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
    let restricted = Restricted::sealed(&owner).await?;
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

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn adding_the_bind_checks_keeps_session_readers_moving(owner: PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "ALTER TABLE sessions DROP CONSTRAINT sessions_chat_bind_size;
         ALTER TABLE login_requests DROP CONSTRAINT login_requests_chat_bind_size;
         ALTER TABLE link_requests DROP CONSTRAINT link_requests_chat_bind_size;",
    )
    .execute(&owner)
    .await?;
    let mut blocker = owner.begin().await?;
    sqlx::query("SELECT count(*) FROM sessions")
        .execute(&mut *blocker)
        .await?;
    let migration = tokio::spawn({
        let owner = owner.clone();
        async move {
            sqlx::raw_sql(include_str!(
                "../../migrations/0319_chat_bind_size_checks.sql"
            ))
            .execute(&owner)
            .await
            .map(|_| ())
        }
    });
    let mut reader = owner.acquire().await?;
    sqlx::raw_sql("SET lock_timeout = '100ms'")
        .execute(&mut *reader)
        .await?;

    let watched = Instant::now();
    let mut slowest = Duration::ZERO;
    let mut timeouts = 0;
    while watched.elapsed() < Duration::from_millis(2_500) {
        let read = Instant::now();
        if let Err(error) = sqlx::query("SELECT 1 FROM sessions LIMIT 1")
            .execute(&mut *reader)
            .await
        {
            let code = error.as_database_error().and_then(|error| error.code());
            anyhow::ensure!(code.as_deref() == Some("55P03"), "{error}");
            timeouts += 1;
        }
        slowest = slowest.max(read.elapsed());
    }
    blocker.commit().await?;
    migration.await??;
    let added: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_catalog.pg_constraint WHERE conname LIKE '%\\_chat\\_bind\\_size'",
    )
    .fetch_one(&owner)
    .await?;

    assert_eq!(
        (timeouts, added),
        (0, 3),
        "slowest session read {slowest:?}"
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_migration_lock_cancels_only_a_plain_autovacuum(owner: PgPool) -> anyhow::Result<()> {
    let vacuum = Some("autovacuum: VACUUM public.sessions");
    let wraparound = Some("autovacuum: VACUUM public.sessions (to prevent wraparound)");
    let cases = [
        (vec!["autovacuum worker"], vec![vacuum], true),
        (
            vec!["autovacuum worker", "autovacuum worker"],
            vec![vacuum, Some("autovacuum: ANALYZE public.sessions")],
            true,
        ),
        (
            vec!["autovacuum worker"],
            vec![Some("<insufficient privilege>")],
            false,
        ),
        (vec!["autovacuum worker"], vec![wraparound], false),
        (
            vec!["autovacuum worker", "autovacuum worker"],
            vec![vacuum, wraparound],
            false,
        ),
        (
            vec!["autovacuum worker", "client backend"],
            vec![vacuum, Some("SELECT 1")],
            false,
        ),
        (vec!["autovacuum worker"], vec![None], false),
        (Vec::new(), Vec::new(), false),
    ];
    for (types, queries, expected) in cases {
        let cancellable: bool = sqlx::query_scalar("SELECT public.migration_cancellable($1, $2)")
            .bind(&types)
            .bind(&queries)
            .fetch_one(&owner)
            .await?;
        assert_eq!(cancellable, expected, "{types:?} {queries:?}");
    }
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_migration_lock_never_cancels_a_client_blocker(owner: PgPool) -> anyhow::Result<()> {
    let mut client = owner.begin().await?;
    let client_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *client)
        .await?;
    sqlx::query("SELECT count(*) FROM sessions")
        .execute(&mut *client)
        .await?;
    let busy = tokio::spawn(async move {
        let slept = sqlx::query("SELECT pg_sleep(2)")
            .execute(&mut *client)
            .await;
        client.commit().await?;
        slept.map(|_| ())
    });
    let running = Instant::now();
    while !sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid = $1 AND state = 'active' AND query LIKE 'SELECT pg_sleep%')",
    )
    .bind(client_pid)
    .fetch_one(&owner)
    .await?
    {
        assert!(running.elapsed() < Duration::from_secs(5), "the client query never started");
        tokio::task::yield_now().await;
    }

    let mut migration = owner.begin().await?;
    let refused = sqlx::query(
        "SELECT public.migration_lock_tables(ARRAY['public.sessions']::regclass[], 'ACCESS EXCLUSIVE', 2)",
    )
    .execute(&mut *migration)
    .await
    .expect_err("a client blocker is waited for, never cancelled");
    migration.rollback().await?;

    let code = refused
        .as_database_error()
        .and_then(|error| error.code())
        .map(|code| code.into_owned());
    assert_eq!(code.as_deref(), Some("55P03"), "{refused}");
    assert!(
        refused.to_string().contains(&format!(
            "pid {client_pid} client backend AccessShareLock held"
        )),
        "the last error names the blocker: {refused}"
    );
    busy.await?
        .map_err(|error| anyhow::anyhow!("the client query was cancelled: {error}"))?;
    Ok(())
}
