use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode, header};
use chrono::Utc;
use equix::{EquiXBuilder, RuntimeOption};
use sc_chat_wire::bind::make_bind;
use sc_chat_wire::pow::{POW_V, challenge, meets_effort};
use sc_chat_wire::prost::Message;
use sc_chat_wire::proto::{Bind, Inner, InnerResponse, Pow, Tier};
use sc_chat_wire::random_bytes;
use sc_chat_wire::seal::{RequestSealer, SealedRequest};
use sc_chat_wire::sign::{public_key, sign_request};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::modules::auth::LinkService;
use crate::modules::auth::sealed::config::SealedCfg;
use crate::modules::auth::sealed::{Sealed, router};
use crate::modules::chat::channel::ock::OckRing;
use crate::modules::chat::test_support::{chat_state, vector_ock};

pub const PEER: [u8; 4] = [198, 51, 100, 7];
pub const SEED_KEY: [u8; 32] = [9; 32];

pub struct Device {
    pub id: Uuid,
    pub seed: [u8; 32],
    pub box_pub: [u8; 32],
}

impl Device {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            seed: random(),
            box_pub: random(),
        }
    }

    pub fn bind(&self) -> Bind {
        make_bind(&self.id.to_string(), &self.seed, &self.box_pub, now_ms())
    }

    pub fn auth_pub(&self) -> [u8; 32] {
        public_key(&self.seed)
    }
}

pub fn random() -> [u8; 32] {
    random_bytes().expect("randomness")
}

pub fn now_ms() -> u64 {
    u64::try_from(Utc::now().timestamp_millis()).expect("a positive clock")
}

pub fn cfg() -> SealedCfg {
    SealedCfg {
        enabled: true,
        plain: false,
        pow: true,
        keys_dir: PathBuf::from("/nonexistent"),
        ock_current: 1,
        node: "main".to_owned(),
        free_per_min: 120,
        pow_per_min: 1000,
        pow_floor: 1,
        pow_cap: 64,
        trusted_proxies: Vec::new(),
    }
}

pub fn redis() -> anyhow::Result<deadpool_redis::Pool> {
    let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
    Ok(crate::redis::connect_url(&url)?)
}

pub fn sealed(cfg: SealedCfg, pool: &PgPool) -> anyhow::Result<Arc<Sealed>> {
    let redis = redis()?;
    let chat = chat_state(pool.clone(), redis.clone(), true);
    Ok(Sealed::new(
        cfg,
        OckRing::new(1, BTreeMap::from([(1, vector_ock())]))?,
        SEED_KEY,
        redis,
        chat.auth.clone(),
        LinkService::new(pool.clone(), chat.auth.clone()),
    ))
}

pub fn app(sealed: &Arc<Sealed>) -> Router {
    router(Some(sealed.clone()))
}

pub async fn oauth_app(pool: &PgPool) -> anyhow::Result<()> {
    let app = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO oauth_apps (id, name, client_id, client_secret, redirect_uri)
         VALUES ($1, 'test', $2, 'secret', 'https://app.test/')",
    )
    .bind(app)
    .bind(app.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn session_for(pool: &PgPool, user: &str, bind: Option<&Device>) -> anyhow::Result<Uuid> {
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

pub fn call(method: &str, path: &str, query: &str, device: &Device) -> Inner {
    Inner {
        ts_ms: now_ms(),
        method: method.to_owned(),
        path: path.to_owned(),
        query: query.to_owned(),
        tier: Tier::Direct as i32,
        bind: Some(device.bind()),
        ..Inner::default()
    }
}

pub fn login(device: &Device) -> Inner {
    call("GET", "/auth/login", "", device)
}

pub fn link_create(device: &Device, mode: &str) -> Inner {
    let mut inner = call("POST", "/auth/link/create", "", device);
    inner.body = json!({ "mode": mode }).to_string().into_bytes();
    inner
}

pub fn marker(mut inner: Inner) -> Inner {
    inner.pow = Some(Pow {
        v: POW_V,
        ..Pow::default()
    });
    inner
}

pub fn seal(mut inner: Inner, device: &Device) -> SealedRequest {
    let sealer = RequestSealer::new(1, &vector_ock().public(), random()).expect("a sealer");
    sign_request(&device.seed, &mut inner, sealer.enc());
    sealer
        .seal(&inner.encode_to_vec())
        .expect("a sealed request")
}

pub fn from(peer: [u8; 4]) -> Option<SocketAddr> {
    Some(SocketAddr::from((peer, 40_000)))
}

pub async fn exchange(
    app: &Router,
    request: &SealedRequest,
    peer: Option<SocketAddr>,
) -> (u32, Value) {
    let mut http =
        Request::post("/auth/x").header(header::CONTENT_TYPE, "application/octet-stream");
    if let Some(peer) = peer {
        http = http.extension(ConnectInfo(peer));
    }
    let response = app
        .clone()
        .oneshot(
            http.body(Body::from(request.envelope.clone()))
                .expect("a request"),
        )
        .await
        .expect("an answer");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("a body");
    let plain = request
        .response_key
        .open(&request.enc, &body)
        .expect("the answer opens");
    let answer = InnerResponse::decode(plain.as_slice()).expect("an inner answer");
    let json = serde_json::from_slice(&answer.body).unwrap_or(Value::Null);
    (answer.status, json)
}

pub async fn send(app: &Router, inner: Inner, device: &Device) -> (u32, Value) {
    exchange(app, &seal(inner, device), from(PEER)).await
}

pub fn solve(seed: &[u8], effort: u32, auth_pub: &[u8; 32], path: &str) -> Pow {
    let seed: [u8; 32] = seed.try_into().expect("a 32 byte seed");
    let mut builder = EquiXBuilder::new();
    builder.runtime(RuntimeOption::InterpretOnly);
    for counter in 0u128.. {
        let nonce = counter.to_be_bytes();
        let bytes = challenge(&seed, auth_pub, path, &nonce, effort);
        let Ok(equix) = builder.build(&bytes) else {
            continue;
        };
        for solution in equix.solve() {
            let solution = solution.to_bytes();
            if meets_effort(&bytes, &solution, effort) {
                return Pow {
                    v: POW_V,
                    seed: seed.to_vec(),
                    effort,
                    nonce: nonce.to_vec(),
                    solution: solution.to_vec(),
                };
            }
        }
    }
    unreachable!("the nonce space is not exhausted")
}

pub fn with_pow(mut inner: Inner, pow: Pow) -> Inner {
    inner.pow = Some(pow);
    inner
}
