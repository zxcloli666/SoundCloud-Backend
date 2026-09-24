use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::{CACHE_CONTROL, RETRY_AFTER};
use axum::http::{HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use deadpool_redis::Pool as RedisPool;
use deadpool_redis::redis::Script;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use tracing::warn;

use crate::config::{AdmissionCfg, AdmissionLimitCfg};

const ADMISSION_SCRIPT: &str = r#"
local window = tonumber(ARGV[3])

local function remaining(key)
    local ttl = redis.call('PTTL', key)
    if ttl == -1 then
        redis.call('PEXPIRE', key, window)
        return window
    end
    return math.max(ttl, 1)
end

local global_count = tonumber(redis.call('GET', KEYS[1]) or '0')
if global_count >= tonumber(ARGV[1]) then
    return {0, remaining(KEYS[1])}
end

local client_count = tonumber(redis.call('GET', KEYS[2]) or '0')
if client_count >= tonumber(ARGV[2]) then
    return {0, remaining(KEYS[2])}
end

global_count = redis.call('INCR', KEYS[1])
if global_count == 1 then
    redis.call('PEXPIRE', KEYS[1], window)
end

client_count = redis.call('INCR', KEYS[2])
if client_count == 1 then
    redis.call('PEXPIRE', KEYS[2], window)
end

return {1, 0}
"#;

static SCRIPT: LazyLock<Script> = LazyLock::new(|| Script::new(ADMISSION_SCRIPT));
const WARNING_INTERVAL_MILLISECONDS: u64 = 30_000;

pub struct PublicAdmission {
    redis: RedisPool,
    config: AdmissionCfg,
    in_flight: Semaphore,
    namespace: String,
    last_warning_at: AtomicU64,
}

#[derive(Clone, Copy, Debug)]
enum Endpoint {
    Login,
    LinkCreate,
    Resolve,
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Allowed,
    Limited { retry_after_seconds: u64 },
    Unavailable,
}

#[derive(Debug, thiserror::Error)]
enum StoreError {
    #[error("Redis pool error: {0}")]
    Pool(#[from] deadpool_redis::PoolError),
    #[error("Redis command error: {0}")]
    Redis(#[from] deadpool_redis::redis::RedisError),
}

impl PublicAdmission {
    pub fn new(redis: RedisPool, config: AdmissionCfg) -> Arc<Self> {
        Self::with_namespace(redis, config, "public:admission")
    }

    fn with_namespace(
        redis: RedisPool,
        config: AdmissionCfg,
        namespace: impl Into<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            redis,
            in_flight: Semaphore::new(config.max_in_flight),
            config,
            namespace: namespace.into(),
            last_warning_at: AtomicU64::new(0),
        })
    }

    async fn check(&self, endpoint: Endpoint, address: SocketAddr) -> Decision {
        let Ok(_permit) = self.in_flight.try_acquire() else {
            return Decision::Unavailable;
        };
        match tokio::time::timeout(self.config.timeout, self.check_redis(endpoint, address)).await {
            Ok(Ok(decision)) => decision,
            Ok(Err(error)) => {
                if self.warning_due() {
                    warn!(endpoint = endpoint.name(), %error, "Auth admission store failed");
                }
                Decision::Unavailable
            }
            Err(_) => {
                if self.warning_due() {
                    warn!(
                        endpoint = endpoint.name(),
                        timeout_ms = self.config.timeout.as_millis(),
                        "Auth admission store timed out"
                    );
                }
                Decision::Unavailable
            }
        }
    }

    async fn check_redis(
        &self,
        endpoint: Endpoint,
        address: SocketAddr,
    ) -> Result<Decision, StoreError> {
        let limits = self.limits(endpoint);
        let global_key = format!("{}:{{{}}}:global", self.namespace, endpoint.key());
        let client_key = format!(
            "{}:{{{}}}:client:{}",
            self.namespace,
            endpoint.key(),
            client_identity(address.ip())
        );
        let window_milliseconds = i64::try_from(self.config.window.as_millis()).unwrap_or(i64::MAX);
        let mut connection = self.redis.get().await?;
        let (allowed, retry_after_milliseconds): (i64, i64) = SCRIPT
            .key(global_key)
            .key(client_key)
            .arg(limits.global)
            .arg(limits.per_client)
            .arg(window_milliseconds)
            .invoke_async(&mut connection)
            .await?;

        if allowed == 1 {
            return Ok(Decision::Allowed);
        }
        Ok(Decision::Limited {
            retry_after_seconds: milliseconds_to_seconds(retry_after_milliseconds),
        })
    }

    fn limits(&self, endpoint: Endpoint) -> AdmissionLimitCfg {
        match endpoint {
            Endpoint::Login => self.config.login,
            Endpoint::LinkCreate => self.config.link_create,
            Endpoint::Resolve => self.config.resolve,
        }
    }

    fn warning_due(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let mut previous = self.last_warning_at.load(Ordering::Relaxed);
        loop {
            if previous != 0 && now >= previous && now - previous < WARNING_INTERVAL_MILLISECONDS {
                return false;
            }
            match self.last_warning_at.compare_exchange_weak(
                previous,
                now,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(current) => previous = current,
            }
        }
    }
}

impl Endpoint {
    fn key(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::LinkCreate => "link-create",
            Self::Resolve => "resolve",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Login => "/auth/login",
            Self::LinkCreate => "/auth/link/create",
            Self::Resolve => "/resolve",
        }
    }
}

pub(crate) async fn admit_login(
    State(limiter): State<Arc<PublicAdmission>>,
    request: Request,
    next: Next,
) -> Response {
    admit(Endpoint::Login, limiter, request, next).await
}

pub(crate) async fn admit_link_create(
    State(limiter): State<Arc<PublicAdmission>>,
    request: Request,
    next: Next,
) -> Response {
    admit(Endpoint::LinkCreate, limiter, request, next).await
}

pub(crate) async fn admit_resolve(
    State(limiter): State<Arc<PublicAdmission>>,
    request: Request,
    next: Next,
) -> Response {
    admit(Endpoint::Resolve, limiter, request, next).await
}

async fn admit(
    endpoint: Endpoint,
    limiter: Arc<PublicAdmission>,
    request: Request,
    next: Next,
) -> Response {
    let Some(address) = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connection| connection.0)
    else {
        if limiter.warning_due() {
            warn!(
                endpoint = endpoint.name(),
                "Auth admission has no transport address"
            );
        }
        return rejection(AdmissionRejection::Unavailable, 1);
    };

    match limiter.check(endpoint, address).await {
        Decision::Allowed => next.run(request).await,
        Decision::Limited {
            retry_after_seconds,
        } => rejection(AdmissionRejection::Limited, retry_after_seconds),
        Decision::Unavailable => rejection(AdmissionRejection::Unavailable, 1),
    }
}

#[derive(Clone, Copy)]
enum AdmissionRejection {
    Limited,
    Unavailable,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RejectionBody {
    status_code: u16,
    code: &'static str,
    message: &'static str,
}

fn rejection(kind: AdmissionRejection, retry_after_seconds: u64) -> Response {
    let (status, code, message) = match kind {
        AdmissionRejection::Limited => (
            StatusCode::TOO_MANY_REQUESTS,
            "auth_admission_limited",
            "Too many authentication requests",
        ),
        AdmissionRejection::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "auth_admission_unavailable",
            "Authentication is temporarily unavailable",
        ),
    };
    let mut response = (
        status,
        Json(RejectionBody {
            status_code: status.as_u16(),
            code,
            message,
        }),
    )
        .into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store, private"));
    if let Ok(value) = HeaderValue::from_str(&retry_after_seconds.max(1).to_string()) {
        response.headers_mut().insert(RETRY_AFTER, value);
    }
    response
}

fn client_identity(address: IpAddr) -> String {
    let mut hash = Sha256::new();
    match address {
        IpAddr::V4(address) => {
            hash.update([4]);
            hash.update(address.octets());
        }
        IpAddr::V6(address) => {
            if let Some(address) = address.to_ipv4_mapped() {
                hash.update([4]);
                hash.update(address.octets());
            } else {
                hash.update([6]);
                hash.update(&address.octets()[..8]);
            }
        }
    }
    hex::encode(hash.finalize())
}

fn milliseconds_to_seconds(milliseconds: i64) -> u64 {
    u64::try_from(milliseconds.max(1))
        .unwrap_or(u64::MAX)
        .saturating_add(999)
        / 1_000
}

#[cfg(test)]
#[path = "admission/tests.rs"]
mod tests;
