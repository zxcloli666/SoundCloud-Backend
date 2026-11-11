mod nats;

#[cfg(test)]
pub(crate) mod memory;

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::future::{BoxFuture, join_all};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::task::JoinHandle;

pub(crate) use nats::NatsSpend;

use crate::config::{CapSettings, micro_usd};

const READ_BUDGET: Duration = Duration::from_millis(800);
const WRITE_BUDGET: Duration = Duration::from_secs(5);
const MAX_PLAIN_USER_LEN: usize = 64;
const HASHED_USER_LEN: usize = 32;

pub(crate) trait SpendStore: Send + Sync {
    fn spent(&self, key: String) -> BoxFuture<'_, Result<u64, SpendError>>;

    fn add(&self, key: String, micro_usd: u64) -> BoxFuture<'_, Result<u64, SpendError>>;
}

#[derive(Debug, Error)]
pub(crate) enum SpendError {
    #[error("the spend bucket is unreachable: {0}")]
    Unreachable(String),
    #[error("a spend counter holds something that is not a number")]
    Corrupt,
    #[error("a spend counter stayed contended after every retry")]
    Contended,
}

pub(crate) struct Budget {
    store: Arc<dyn SpendStore>,
    ladder: String,
    caps: Caps,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Caps {
    month: Option<u64>,
    day: Option<u64>,
    user_month: Option<u64>,
    user_day: Option<u64>,
}

impl Budget {
    pub fn new(store: Arc<dyn SpendStore>, ladder: &str, caps: Caps) -> Self {
        Self {
            store,
            ladder: ladder.to_owned(),
            caps,
        }
    }

    pub async fn is_spent(&self, user: Option<&str>, now: DateTime<Utc>) -> bool {
        let counters = self.counters(user, now);
        let reads = counters
            .iter()
            .map(|(key, _)| self.store.spent(key.clone()));
        let Ok(spent) = tokio::time::timeout(READ_BUDGET, join_all(reads)).await else {
            tracing::warn!(ladder = %self.ladder, "spend counters did not answer in time, priced providers are skipped");
            return true;
        };
        counters
            .iter()
            .zip(spent)
            .any(|((_, cap), spent)| match spent {
                Ok(spent) => spent >= *cap,
                Err(error) => {
                    tracing::warn!(ladder = %self.ladder, %error, "spend counter is unreadable, priced providers are skipped");
                    true
                }
            })
    }

    pub fn charge(
        self: &Arc<Self>,
        user: Option<&str>,
        micro_usd: u64,
        now: DateTime<Utc>,
    ) -> Option<JoinHandle<()>> {
        let runtime = tokio::runtime::Handle::try_current().ok()?;
        let keys: Vec<String> = self
            .counters(user, now)
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        if micro_usd == 0 || keys.is_empty() {
            return None;
        }
        let budget = self.clone();
        Some(runtime.spawn(async move {
            let writes = keys
                .into_iter()
                .map(|key| budget.store.add(key, micro_usd));
            match tokio::time::timeout(WRITE_BUDGET, join_all(writes)).await {
                Ok(written) => {
                    for error in written.into_iter().filter_map(Result::err) {
                        tracing::warn!(ladder = %budget.ladder, micro_usd, %error, "spend was not counted");
                    }
                }
                Err(_) => {
                    tracing::warn!(ladder = %budget.ladder, micro_usd, "spend was not counted in time");
                }
            }
        }))
    }

    fn counters(&self, user: Option<&str>, now: DateTime<Utc>) -> Vec<(String, u64)> {
        let month = now.format("%Y-%m");
        let day = now.format("%Y-%m-%d");
        let ladder = &self.ladder;
        let user = user.map(user_token);
        let mut counters = Vec::with_capacity(4);
        if let Some(cap) = self.caps.month {
            counters.push((format!("{ladder}.month.{month}"), cap));
        }
        if let Some(cap) = self.caps.day {
            counters.push((format!("{ladder}.day.{day}"), cap));
        }
        if let (Some(cap), Some(user)) = (self.caps.user_month, &user) {
            counters.push((format!("{ladder}.user.{user}.month.{month}"), cap));
        }
        if let (Some(cap), Some(user)) = (self.caps.user_day, &user) {
            counters.push((format!("{ladder}.user.{user}.day.{day}"), cap));
        }
        counters
    }
}

impl Caps {
    pub fn is_set(&self) -> bool {
        *self != Self::default()
    }
}

impl From<CapSettings> for Caps {
    fn from(settings: CapSettings) -> Self {
        Self {
            month: settings.month_usd.map(micro_usd),
            day: settings.day_usd.map(micro_usd),
            user_month: settings.user_month_usd.map(micro_usd),
            user_day: settings.user_day_usd.map(micro_usd),
        }
    }
}

fn user_token(user: &str) -> String {
    let plain = user
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    if plain && !user.is_empty() && user.len() <= MAX_PLAIN_USER_LEN {
        return user.to_owned();
    }
    let digest = hex::encode(Sha256::digest(user.as_bytes()));
    format!("h{}", &digest[..HASHED_USER_LEN])
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
