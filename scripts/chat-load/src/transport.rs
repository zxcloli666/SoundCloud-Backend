use std::time::Duration;

use bytes::Bytes;
use reqwest::header::{CONTENT_TYPE, RETRY_AFTER};

use crate::pro;
use crate::route::{Kind, Route};

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const IDLE_KEPT: Duration = Duration::from_secs(120);
pub const PRO_PIECE: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub struct Outer {
    pub status: u16,
    pub retry_after: Option<Duration>,
    pub body: Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    Connect,
    Timeout,
    Link(String),
    Pro(u16),
}

impl Failure {
    pub fn label(&self) -> String {
        match self {
            Failure::Connect => "connect".to_owned(),
            Failure::Timeout => "timeout".to_owned(),
            Failure::Link(_) => "link".to_owned(),
            Failure::Pro(status) => format!("pro_{status}"),
        }
    }

    pub fn from_reqwest(error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            Failure::Timeout
        } else if error.is_connect() {
            Failure::Connect
        } else {
            Failure::Link(error.to_string())
        }
    }
}

pub struct Transport {
    client: reqwest::Client,
    kind: Kind,
    base: String,
    target: String,
}

impl Transport {
    pub fn new(route: &Route, origin: &str) -> anyhow::Result<Self> {
        let builder = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .tcp_nodelay(true)
            .no_proxy();
        let builder = match route.kind {
            Kind::Pro => builder.pool_max_idle_per_host(0).http1_only(),
            Kind::Direct | Kind::Relay => {
                let pooled = builder
                    .pool_max_idle_per_host(1)
                    .pool_idle_timeout(IDLE_KEPT);
                match route.base.starts_with("http://") {
                    true => pooled.http2_prior_knowledge(),
                    false => pooled,
                }
            }
        };
        Ok(Self {
            client: builder.build()?,
            kind: route.kind,
            base: route.base.clone(),
            target: format!("{origin}/chat/x"),
        })
    }

    pub async fn exchange(&self, envelope: Vec<u8>, timeout: Duration) -> Result<Outer, Failure> {
        match self.kind {
            Kind::Pro => {
                pro::exchange(
                    &self.client,
                    &self.base,
                    &self.target,
                    envelope,
                    timeout,
                    PRO_PIECE,
                )
                .await
            }
            Kind::Direct | Kind::Relay => self.post(envelope, timeout).await,
        }
    }

    async fn post(&self, envelope: Vec<u8>, timeout: Duration) -> Result<Outer, Failure> {
        let response = self
            .client
            .post(format!("{}/chat/x", self.base))
            .header(CONTENT_TYPE, "application/octet-stream")
            .timeout(timeout)
            .body(envelope)
            .send()
            .await
            .map_err(|error| Failure::from_reqwest(&error))?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse().ok())
            .map(Duration::from_secs);
        let body = response
            .bytes()
            .await
            .map_err(|error| Failure::from_reqwest(&error))?;
        Ok(Outer {
            status,
            retry_after,
            body,
        })
    }
}
