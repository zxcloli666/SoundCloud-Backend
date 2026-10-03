use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use tracing::{debug, info};

use super::gate::LiveGate;
use super::query::{LiveClass, LiveKind, LiveQuery, PROXY_MIN_LEFT};
use crate::error::AppError;
use crate::sc::errors::ScFailure;
use crate::sc::{ScReadService, classify, retry_after_seconds};

#[derive(Debug, PartialEq)]
pub enum Fetched {
    Items(Vec<Value>),
    Empty,
    RateLimited(Option<i64>),
    Unavailable,
    Timeout,
    Untried,
}

impl Fetched {
    fn from_items(items: Vec<Value>) -> Self {
        if items.is_empty() {
            Self::Empty
        } else {
            Self::Items(items)
        }
    }

    pub fn outcome(&self) -> &'static str {
        match self {
            Self::Items(_) => "ok",
            Self::Empty => "empty",
            Self::RateLimited(_) => "rate_limited",
            Self::Unavailable | Self::Untried => "unavailable",
            Self::Timeout => "timeout",
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Items(items) => items.len(),
            _ => 0,
        }
    }
}

pub struct Fetch<'a> {
    pub read: &'a ScReadService,
    pub gate: &'a LiveGate,
    pub kind: LiveKind,
    pub class: LiveClass,
    pub query: &'a LiveQuery,
}

impl Fetch<'_> {
    pub async fn run(&self) -> Fetched {
        let started = Instant::now();
        let on_proxy = AtomicBool::new(false);
        match tokio::time::timeout(self.class.budget(), self.tiers(started, &on_proxy)).await {
            Ok(fetched) => fetched,
            Err(_) => {
                let tier = if on_proxy.load(Ordering::Relaxed) {
                    "proxy"
                } else {
                    "relay"
                };
                self.report(tier, &Fetched::Timeout, started);
                Fetched::Timeout
            }
        }
    }

    async fn tiers(&self, started: Instant, on_proxy: &AtomicBool) -> Fetched {
        let limit = self.class.window_size(self.kind);
        let wait = self.class.relay_wait();
        let mut missed = Fetched::Untried;
        if self.read.relay_usable().await {
            let relay = self
                .read
                .search_relay(self.kind.search_type(), &self.query.text, limit, wait)
                .await;
            if let Some(items) = relay {
                let fetched = Fetched::from_items(items);
                self.report("relay", &fetched, started);
                return fetched;
            }
            missed = relay_miss(started.elapsed(), wait);
            self.report("relay", &missed, started);
        }
        let left = self.class.budget().saturating_sub(started.elapsed());
        if !self.class.proxy_allowed() || left < PROXY_MIN_LEFT || !self.gate.proxy_admits().await {
            return missed;
        }
        on_proxy.store(true, Ordering::Relaxed);
        let fetched = match self
            .read
            .search_proxy(self.kind.search_type(), &self.query.text, limit, left)
            .await
        {
            Ok(items) => Fetched::from_items(items),
            Err(error) => proxy_failure(&error),
        };
        self.report("proxy", &fetched, started);
        fetched
    }

    fn report(&self, tier: &'static str, fetched: &Fetched, started: Instant) {
        crate::metrics::record_live_fetch(self.kind.as_str(), tier, fetched.outcome());
        info!(
            kind = self.kind.as_str(),
            class = self.class.as_str(),
            tier,
            outcome = fetched.outcome(),
            items = fetched.len(),
            ms = started.elapsed().as_millis() as u64,
            q_len = self.query.text.chars().count(),
            q_hash = self.query.short_hash(),
            "live search fetch"
        );
        debug!(q = %self.query.text, tier, "live search fetch query");
    }
}

pub fn relay_miss(elapsed: Duration, wait: Duration) -> Fetched {
    if elapsed >= wait {
        Fetched::Timeout
    } else {
        Fetched::Unavailable
    }
}

pub fn proxy_failure(error: &AppError) -> Fetched {
    match classify(error) {
        ScFailure::RateLimited | ScFailure::Ban => Fetched::RateLimited(retry_after_seconds(error)),
        _ if matches!(error, AppError::ScDeadlineExceeded) => Fetched::Timeout,
        _ => Fetched::Unavailable,
    }
}
