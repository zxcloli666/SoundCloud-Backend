use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::layout::Identity;
use crate::route::{Kind, Route};
use crate::transport::{Failure, Transport};
use crate::wire::{self, Call, Ock};

pub const RETRY_AFTER_MS: &str = "retry-after-ms";

#[derive(Debug)]
pub enum Reply {
    Json(Value),
    Empty { retry_after_ms: Option<u64> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallError {
    Transport(Failure),
    Outer {
        status: u16,
        retry_after: Option<Duration>,
    },
    Inner {
        status: u32,
        code: String,
        retry_ms: Option<u64>,
    },
    Broken(String),
}

impl CallError {
    pub fn label(&self) -> String {
        match self {
            CallError::Transport(failure) => failure.label(),
            CallError::Outer { status, .. } => format!("outer_{status}"),
            CallError::Inner { code, .. } => code.clone(),
            CallError::Broken(_) => "broken".to_owned(),
        }
    }

    pub fn wait_hint(&self) -> Option<Duration> {
        match self {
            CallError::Outer { retry_after, .. } => *retry_after,
            CallError::Inner { retry_ms, .. } => retry_ms.map(Duration::from_millis),
            _ => None,
        }
    }

    pub fn is_clock(&self) -> bool {
        matches!(self, CallError::Inner { code, .. } if code == "chat_clock")
    }
}

#[derive(Default)]
pub struct Clock {
    offset_ms: AtomicI64,
}

impl Clock {
    pub fn now_ms(&self) -> u64 {
        let local = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or(0);
        u64::try_from(local + self.offset_ms.load(Ordering::Relaxed)).unwrap_or(0)
    }

    pub fn learn(&self, server_now: i64) {
        let local = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or(0);
        self.offset_ms.store(server_now - local, Ordering::Relaxed);
    }
}

pub struct Client {
    identity: Arc<Identity>,
    ock: Ock,
    transport: Transport,
    kind: Kind,
    clock: Arc<Clock>,
}

impl Client {
    pub fn new(
        identity: Arc<Identity>,
        ock: Ock,
        route: &Route,
        origin: &str,
        clock: Arc<Clock>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            identity,
            ock,
            transport: Transport::new(route, origin)?,
            kind: route.kind,
            clock,
        })
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub async fn call(
        &self,
        method: &str,
        path: &str,
        query: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<Reply, CallError> {
        let call = Call {
            method,
            path,
            query,
            body: body
                .map(|body| body.to_string().into_bytes())
                .unwrap_or_default(),
        };
        let sealed = wire::seal(
            &self.identity,
            &self.ock,
            call,
            self.kind.tier(),
            self.kind.answer_bytes(),
            self.clock.now_ms(),
        )
        .map_err(|error| CallError::Broken(error.to_string()))?;
        let outer = self
            .transport
            .exchange(sealed.envelope.clone(), timeout)
            .await
            .map_err(CallError::Transport)?;
        if outer.status != 200 {
            return Err(CallError::Outer {
                status: outer.status,
                retry_after: outer.retry_after,
            });
        }
        let inner = sealed
            .open(&outer.body)
            .map_err(|error| CallError::Broken(error.to_string()))?;
        match inner.status {
            200 => serde_json::from_slice(&inner.body)
                .map(Reply::Json)
                .map_err(|error| CallError::Broken(error.to_string())),
            204 => Ok(Reply::Empty {
                retry_after_ms: inner
                    .headers
                    .iter()
                    .find(|header| header.name.eq_ignore_ascii_case(RETRY_AFTER_MS))
                    .and_then(|header| header.value.parse().ok()),
            }),
            status => Err(self.inner_error(status, &inner.body)),
        }
    }

    fn inner_error(&self, status: u32, body: &[u8]) -> CallError {
        let parsed: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        if let Some(server_now) = parsed["server_now"].as_i64() {
            self.clock.learn(server_now);
        }
        CallError::Inner {
            status,
            code: parsed["error"]
                .as_str()
                .map_or_else(|| format!("inner_{status}"), str::to_owned),
            retry_ms: parsed["retry_ms"].as_u64(),
        }
    }
}
