use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use chrono::Utc;
use sc_chat_wire::bind::make_bind;
use sc_chat_wire::prost::Message;
use sc_chat_wire::proto::{Bind, Inner, InnerResponse, Tier};
use sc_chat_wire::random_bytes;
use sc_chat_wire::seal::{RequestSealer, ResponseKey, SealedRequest};
use sc_chat_wire::sign::{public_key, sign_request};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;
use uuid::Uuid;

use super::{Channel, router};
use crate::common::session::session_hash;
use crate::modules::chat::model::DeviceRow;
use crate::modules::chat::test_support::{chat_state, offline_chat_state, vector_ock};
use crate::modules::chat::{ChatError, ChatState, devices, routes};

const PEER: [u8; 4] = [198, 51, 100, 7];

#[path = "channel_herd_tests.rs"]
mod herd;

#[path = "channel_logout_tests.rs"]
mod logout;

struct Device {
    id: Uuid,
    seed: [u8; 32],
    box_pub: [u8; 32],
}

impl Device {
    fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            seed: random(),
            box_pub: random(),
        }
    }

    fn bind(&self) -> Bind {
        make_bind(&self.id.to_string(), &self.seed, &self.box_pub, now_ms())
    }

    fn row(&self, user: &str, session: Uuid, status: &str) -> DeviceRow {
        DeviceRow {
            id: self.id,
            user_id: user.to_owned(),
            session_hash: session_hash(session),
            status: status.to_owned(),
            auth_pub: public_key(&self.seed).to_vec(),
            box_pub: self.box_pub.to_vec(),
            platform: "linux".to_owned(),
            name: "test".to_owned(),
            app_version: "0.0.0".to_owned(),
            caps: Vec::new(),
            tier: "direct".to_owned(),
            created_at: Utc::now(),
            history_unlocked_at: None,
            last_seen_at: Utc::now(),
            revoked_at: None,
        }
    }
}

fn random() -> [u8; 32] {
    random_bytes().expect("randomness")
}

fn now_ms() -> u64 {
    u64::try_from(Utc::now().timestamp_millis()).expect("a positive clock")
}

fn call(method: &str, path: &str, query: &str) -> Inner {
    Inner {
        ts_ms: now_ms(),
        method: method.to_owned(),
        path: path.to_owned(),
        query: query.to_owned(),
        tier: Tier::Direct as i32,
        ..Inner::default()
    }
}

fn from_device(mut inner: Inner, session: Uuid, device: &Device) -> Inner {
    inner.session_id = Some(session.to_string());
    inner.device_id = Some(device.id.to_string());
    inner
}

fn with_bind(mut inner: Inner, device: &Device) -> Inner {
    inner.bind = Some(device.bind());
    inner
}

fn seal(mut inner: Inner, signer: &[u8; 32]) -> SealedRequest {
    let sealer = RequestSealer::new(1, &vector_ock().public(), random()).expect("a sealer");
    sign_request(signer, &mut inner, sealer.enc());
    sealer
        .seal(&inner.encode_to_vec())
        .expect("a sealed request")
}

async fn post(app: &Router, envelope: Vec<u8>) -> Response {
    let request = Request::post("/chat/x")
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .extension(ConnectInfo(SocketAddr::from((PEER, 40_000))));
    app.clone()
        .oneshot(request.body(Body::from(envelope)).expect("a request"))
        .await
        .expect("an answer")
}

async fn sealed_body(app: &Router, request: &SealedRequest) -> Vec<u8> {
    body_of(post(app, request.envelope.clone()).await).await
}

async fn body_of(response: Response) -> Vec<u8> {
    assert_eq!(response.status(), StatusCode::OK);
    to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("a body")
        .to_vec()
}

fn opened(request: &SealedRequest, body: &[u8]) -> (u32, Value) {
    let plain = request
        .response_key
        .open(&request.enc, body)
        .expect("the answer opens");
    let answer = InnerResponse::decode(plain.as_slice()).expect("an inner answer");
    let json = serde_json::from_slice(&answer.body).unwrap_or(Value::Null);
    (answer.status, json)
}

async fn exchange(app: &Router, request: &SealedRequest) -> (u32, Value) {
    let body = sealed_body(app, request).await;
    opened(request, &body)
}

fn offline_app() -> Router {
    routes::public(offline_chat_state(true))
}

fn error(code: ChatError) -> (u32, Value) {
    (u32::from(code.status().as_u16()), code.body())
}

fn redis() -> anyhow::Result<deadpool_redis::Pool> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    Ok(crate::redis::connect_url(&url)?)
}

async fn session_for(pool: &PgPool, user: &str, bind: Option<&Device>) -> anyhow::Result<Uuid> {
    let connection = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO soundcloud_connections
            (id, soundcloud_user_id, username, access_token, refresh_token, expires_at, scope)
         VALUES ($1, $2, $2, 'access', 'refresh', now() + interval '1 hour', '')",
    )
    .bind(connection)
    .bind(user)
    .execute(pool)
    .await?;
    let session = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO sessions (id, soundcloud_connection_id, chat_bind) VALUES ($1, $2, $3)",
    )
    .bind(session)
    .bind(connection)
    .bind(bind.map(|device| device.bind().encode_to_vec()))
    .execute(pool)
    .await?;
    Ok(session)
}

async fn counting(pool: &PgPool, acquired: Arc<AtomicUsize>) -> anyhow::Result<PgPool> {
    let connected = acquired.clone();
    Ok(PgPoolOptions::new()
        .max_connections(2)
        .after_connect(move |_, _| {
            connected.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        })
        .before_acquire(move |_, _| {
            acquired.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(true) })
        })
        .connect_with((*pool.connect_options()).clone())
        .await?)
}

fn remember(chat: &ChatState, row: DeviceRow) {
    chat.devices.keep_row(chat.devices.stamp(), row);
}

fn beta_for(chat: Arc<ChatState>, user: &str) -> Arc<ChatState> {
    let Ok(mut state) = Arc::try_unwrap(chat) else {
        panic!("a fresh chat state has one owner");
    };
    state.config.beta_users.insert(user.to_owned());
    Arc::new(state)
}

#[tokio::test]
async fn a_skewed_clock_gets_a_sealed_chat_clock_with_the_server_time() {
    let app = offline_app();
    let device = Device::new();
    for skew in [-300_000_i64, 300_000] {
        let mut inner = from_device(call("GET", "/chat/config", ""), Uuid::now_v7(), &device);
        inner.ts_ms = now_ms().saturating_add_signed(skew);
        let (status, body) = exchange(&app, &seal(inner, &device.seed)).await;
        assert_eq!(status, 401, "{body}");
        assert_eq!(body["error"], "chat_clock");
        let server_now = body["server_now"].as_i64().expect("server time");
        assert!((server_now - Utc::now().timestamp_millis()).abs() < 5_000);
    }
}

#[tokio::test]
async fn a_forged_or_modified_answer_does_not_open_in_the_client() {
    let app = offline_app();
    let device = Device::new();
    let mut inner = from_device(call("GET", "/chat/config", ""), Uuid::now_v7(), &device);
    inner.ts_ms = 1;
    let request = seal(inner.clone(), &device.seed);
    let body = sealed_body(&app, &request).await;
    assert_eq!(opened(&request, &body).1["error"], "chat_clock");

    let mut modified = body.clone();
    let last = modified.len() - 1;
    modified[last] ^= 1;
    assert!(request.response_key.open(&request.enc, &modified).is_err());

    let relay_key = ResponseKey::from_bytes(random());
    let forged_answer = InnerResponse {
        status: 200,
        headers: Vec::new(),
        body: br#"{"enabled":true}"#.to_vec(),
    };
    let forged = relay_key
        .seal(
            &request.enc,
            random_bytes().expect("a nonce"),
            &forged_answer.encode_to_vec(),
        )
        .expect("a forged answer");
    assert!(request.response_key.open(&request.enc, &forged).is_err());

    let other = seal(inner, &device.seed);
    let other_body = sealed_body(&app, &other).await;
    assert!(
        request
            .response_key
            .open(&request.enc, &other_body)
            .is_err()
    );
}

#[tokio::test]
async fn the_chat_channel_does_not_route_sealed_auth_paths() {
    let app = offline_app();
    let device = Device::new();

    for (method, path) in [
        ("GET", "/auth/login"),
        ("GET", "/auth/login/status"),
        ("POST", "/auth/link/create"),
        ("POST", "/auth/link/claim"),
        ("GET", "/auth/link/status"),
    ] {
        let request = seal(with_bind(call(method, path, ""), &device), &device.seed);
        assert_eq!(
            exchange(&app, &request).await,
            error(ChatError::NotFound),
            "{method} {path}"
        );
    }
}

#[tokio::test]
async fn the_channel_refuses_before_any_lookup_what_it_cannot_route() {
    let app = offline_app();
    let device = Device::new();

    let outside = from_device(call("GET", "/rooms/public", ""), Uuid::now_v7(), &device);
    assert_eq!(
        exchange(&app, &seal(outside, &device.seed)).await,
        error(ChatError::NotFound)
    );

    let mut no_session = call("POST", "/chat/sync", "");
    no_session.device_id = Some(device.id.to_string());
    assert_eq!(
        exchange(&app, &seal(no_session, &device.seed)).await,
        error(ChatError::ReloginRequired)
    );

    let mut no_device = call("POST", "/chat/sync", "");
    no_device.session_id = Some(Uuid::now_v7().to_string());
    assert_eq!(
        exchange(&app, &seal(no_device, &device.seed)).await,
        error(ChatError::Invalid)
    );

    let garbage = {
        let sealer = RequestSealer::new(1, &vector_ock().public(), random()).expect("a sealer");
        sealer.seal(b"\xff\xff\xff not protobuf").expect("sealed")
    };
    assert_eq!(exchange(&app, &garbage).await, error(ChatError::Invalid));
}

#[tokio::test]
async fn a_full_channel_answers_an_outer_busy_with_retry_after() {
    let chat = offline_chat_state(true);
    let channel = Arc::new(Channel::new(chat.clone(), routes::inner().with_state(chat)));
    let app: Router = router(channel.clone());
    let held = channel
        .busy
        .try_acquire_many(u32::try_from(channel.busy.available_permits()).expect("permits"))
        .expect("every permit");
    let device = Device::new();
    let request = seal(
        from_device(call("GET", "/chat/config", ""), Uuid::now_v7(), &device),
        &device.seed,
    );

    let response = post(&app, request.envelope.clone()).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()[header::RETRY_AFTER], "1");

    drop(held);
    let response = post(&app, request.envelope).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn the_same_envelope_is_refused_the_second_time(pool: PgPool) -> anyhow::Result<()> {
    let device = Device::new();
    let session = session_for(&pool, "4101", Some(&device)).await?;
    let app: Router = routes::public(chat_state(pool, redis()?, true));
    let request = seal(
        from_device(call("GET", "/chat/config", ""), session, &device),
        &device.seed,
    );

    let (first, body) = exchange(&app, &request).await;
    assert_eq!(first, 200, "{body}");
    assert_eq!(exchange(&app, &request).await, error(ChatError::Replay));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn an_idle_request_runs_no_pg_query_on_a_warm_cache(pool: PgPool) -> anyhow::Result<()> {
    let device = Device::new();
    let session = session_for(&pool, "4101", Some(&device)).await?;
    let acquired = Arc::new(AtomicUsize::new(0));
    let chat = beta_for(
        chat_state(counting(&pool, acquired.clone()).await?, redis()?, true),
        "4101",
    );
    remember(&chat, device.row("4101", session, "active"));
    let app: Router = routes::public(chat);
    let idle = || {
        seal(
            from_device(call("GET", "/chat/idle", ""), session, &device),
            &device.seed,
        )
    };

    assert_eq!(exchange(&app, &idle()).await, error(ChatError::NotFound));
    let warm = acquired.load(Ordering::SeqCst);
    assert!(warm > 0, "the first call reads the session from PostgreSQL");
    for _ in 0..5 {
        assert_eq!(exchange(&app, &idle()).await, error(ChatError::NotFound));
    }
    assert_eq!(acquired.load(Ordering::SeqCst), warm);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_session_without_a_bind_reaches_only_the_config(pool: PgPool) -> anyhow::Result<()> {
    let session = session_for(&pool, "4102", None).await?;
    let app: Router = routes::public(beta_for(chat_state(pool.clone(), redis()?, true), "4102"));
    let device = Device::new();

    let config = seal(
        from_device(call("GET", "/chat/config", ""), session, &device),
        &device.seed,
    );
    let (status, view) = exchange(&app, &config).await;
    let incarnation: Uuid = sqlx::query_scalar("SELECT incarnation FROM chat.meta")
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, 200, "{view}");
    assert_eq!(view["user_id"], "4102");
    assert_eq!(view["enabled"], true);
    assert_eq!(view["access"], "beta");
    assert_eq!(view["send_paused"], false);
    assert_eq!(view["hold_max_ms"], 20_000);
    assert_eq!(view["ock_current"], 1);
    assert_eq!(view["incarnation"], incarnation.to_string());
    assert_eq!(
        view["limits"],
        json!({
            "text_max": 4096,
            "embeds_max": 3,
            "group_members_max": 200,
            "devices_max": 10,
            "envelope_max": 65536,
            "disappearing_ttls": [3600, 86400, 604800],
        })
    );
    assert!(
        (view["server_now"].as_i64().unwrap_or_default() - Utc::now().timestamp_millis()).abs()
            < 5_000
    );

    let sync = seal(
        from_device(call("POST", "/chat/sync", ""), session, &device),
        &device.seed,
    );
    assert_eq!(
        exchange(&app, &sync).await,
        error(ChatError::ReloginRequired)
    );

    let unknown = seal(
        from_device(call("GET", "/chat/config", ""), Uuid::now_v7(), &device),
        &device.seed,
    );
    assert_eq!(
        exchange(&app, &unknown).await,
        error(ChatError::ReloginRequired)
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn chat_stays_closed_to_users_outside_the_beta(pool: PgPool) -> anyhow::Result<()> {
    let device = Device::new();
    let session = session_for(&pool, "4103", Some(&device)).await?;
    let chat = chat_state(pool, redis()?, true);
    remember(&chat, device.row("4103", session, "active"));
    let app: Router = routes::public(chat);

    let config = seal(
        from_device(call("GET", "/chat/config", ""), session, &device),
        &device.seed,
    );
    let (status, view) = exchange(&app, &config).await;
    assert_eq!((status, view["enabled"].clone()), (200, json!(false)));

    let sync = seal(
        from_device(call("POST", "/chat/sync", ""), session, &device),
        &device.seed,
    );
    assert_eq!(exchange(&app, &sync).await, error(ChatError::Disabled));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_bound_session_answers_only_its_own_device(pool: PgPool) -> anyhow::Result<()> {
    let owner = Device::new();
    let other = Device::new();
    let session = session_for(&pool, "4104", Some(&owner)).await?;
    let chat = beta_for(chat_state(pool, redis()?, true), "4104");
    remember(&chat, owner.row("4104", session, "active"));
    let app: Router = routes::public(chat.clone());
    let idle = |device: &Device, signer: &Device| {
        seal(
            from_device(call("GET", "/chat/idle", ""), session, device),
            &signer.seed,
        )
    };

    assert_eq!(
        exchange(&app, &idle(&owner, &owner)).await,
        error(ChatError::NotFound)
    );
    assert_eq!(
        exchange(&app, &idle(&other, &other)).await,
        error(ChatError::SessionBoundOther)
    );
    assert_eq!(
        exchange(&app, &idle(&owner, &other)).await,
        error(ChatError::Invalid)
    );

    remember(&chat, owner.row("4104", session, "revoked"));
    assert_eq!(
        exchange(&app, &idle(&owner, &owner)).await,
        error(ChatError::DeviceRevoked)
    );

    remember(&chat, owner.row("4999", session, "active"));
    assert_eq!(
        exchange(&app, &idle(&owner, &owner)).await,
        error(ChatError::SessionBoundOther)
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_relogin_with_new_keys_is_read_again_instead_of_refused(
    pool: PgPool,
) -> anyhow::Result<()> {
    let before = Device::new();
    let after = Device::new();
    let session = session_for(&pool, "4105", Some(&before)).await?;
    let chat = beta_for(chat_state(pool.clone(), redis()?, true), "4105");
    remember(&chat, before.row("4105", session, "active"));
    remember(&chat, after.row("4105", session, "active"));
    let app: Router = routes::public(chat);
    let idle = |device: &Device| {
        seal(
            from_device(call("GET", "/chat/idle", ""), session, device),
            &device.seed,
        )
    };
    assert_eq!(
        exchange(&app, &idle(&before)).await,
        error(ChatError::NotFound)
    );

    sqlx::query("UPDATE sessions SET chat_bind = $2 WHERE id = $1")
        .bind(session)
        .bind(after.bind().encode_to_vec())
        .execute(&pool)
        .await?;

    assert_eq!(
        exchange(&app, &idle(&after)).await,
        error(ChatError::NotFound)
    );
    assert_eq!(
        exchange(&app, &idle(&before)).await,
        error(ChatError::SessionBoundOther)
    );
    Ok(())
}

async fn registered(pool: &PgPool, row: &DeviceRow) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO chat.devices
            (id, user_id, session_hash, auth_pub, box_pub, platform, name, app_version, tier)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(row.id)
    .bind(&row.user_id)
    .bind(&row.session_hash)
    .bind(&row.auth_pub)
    .bind(&row.box_pub)
    .bind(&row.platform)
    .bind(&row.name)
    .bind(&row.app_version)
    .bind(&row.tier)
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_revoked_device_is_refused_on_its_next_request(pool: PgPool) -> anyhow::Result<()> {
    let device = Device::new();
    let session = session_for(&pool, "4109", Some(&device)).await?;
    let row = device.row("4109", session, "active");
    registered(&pool, &row).await?;
    let chat = beta_for(chat_state(pool, redis()?, true), "4109");
    let app: Router = routes::public(chat.clone());
    let idle = || {
        seal(
            from_device(call("GET", "/chat/idle", ""), session, &device),
            &device.seed,
        )
    };
    assert_eq!(exchange(&app, &idle()).await, error(ChatError::NotFound));
    assert!(chat.devices.row(device.id).is_some());

    assert_eq!(devices::revoke(&chat, &row).await, Ok(true));
    assert_eq!(
        exchange(&app, &idle()).await,
        error(ChatError::DeviceRevoked)
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_bound_session_without_a_device_reaches_only_config_and_registration(
    pool: PgPool,
) -> anyhow::Result<()> {
    let user = format!("9{}", rand::random::<u32>());
    let device = Device::new();
    let session = session_for(&pool, &user, Some(&device)).await?;
    let app: Router = routes::public(beta_for(chat_state(pool.clone(), redis()?, true), &user));
    let ask = |method: &str, path: &str, body: &str| {
        let mut inner = from_device(call(method, path, ""), session, &device);
        inner.body = body.as_bytes().to_vec();
        seal(inner, &device.seed)
    };

    for (method, path) in [
        ("GET", "/chat/conversations"),
        ("POST", "/chat/sync"),
        ("GET", "/chat/settings"),
        ("GET", "/chat/requests"),
        ("DELETE", "/chat/devices/me"),
    ] {
        assert_eq!(
            exchange(&app, &ask(method, path, "")).await,
            error(ChatError::DeviceRevoked),
            "{method} {path}"
        );
    }
    let (status, config) = exchange(&app, &ask("GET", "/chat/config", "")).await;
    assert_eq!(status, 200, "{config}");

    let register = json!({"platform": "linux", "name": "test", "app_version": "0.0.0"});
    let (status, registered) =
        exchange(&app, &ask("POST", "/chat/devices", &register.to_string())).await;
    assert_eq!(status, 200, "{registered}");
    let (status, listed) = exchange(&app, &ask("GET", "/chat/conversations", "")).await;
    assert_eq!(status, 200, "{listed}");
    Ok(())
}
