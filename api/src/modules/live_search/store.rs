use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::query::{FAIL_TTL, ITEM_TTL, WINDOW_FRESH_SECONDS};
use super::slim::urn_of;
use crate::cache::CacheService;

const PREFIX: &str = "live:v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowState {
    Ok,
    Empty,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    pub at: i64,
    pub state: WindowState,
    pub ids: Vec<String>,
}

impl Window {
    pub fn new(ids: Vec<String>, at: i64) -> Self {
        let state = if ids.is_empty() {
            WindowState::Empty
        } else {
            WindowState::Ok
        };
        Self { at, state, ids }
    }

    pub fn is_fresh(&self, now: i64) -> bool {
        now - self.at < WINDOW_FRESH_SECONDS
    }
}

#[derive(Clone)]
pub struct LiveStore {
    cache: Arc<CacheService>,
}

impl LiveStore {
    pub fn new(cache: Arc<CacheService>) -> Self {
        Self { cache }
    }

    pub async fn window(&self, scope: &str, qh: &str) -> Option<Window> {
        let raw = self.cache.get_raw(&window_key(scope, qh)).await.ok()??;
        serde_json::from_str(&raw).ok()
    }

    pub async fn items(&self, urns: &[String]) -> Vec<Option<Value>> {
        let keys: Vec<String> = urns.iter().map(|urn| item_key(urn)).collect();
        self.cache
            .get_many_raw(&keys)
            .await
            .into_iter()
            .map(|raw| raw.and_then(|raw| serde_json::from_str(&raw).ok()))
            .collect()
    }

    pub async fn item(&self, urn: &str) -> Option<Value> {
        let raw = self.cache.get_raw(&item_key(urn)).await.ok()??;
        serde_json::from_str(&raw).ok()
    }

    pub async fn write(
        &self,
        scope: &str,
        qh: &str,
        window: &Window,
        ttl: u64,
        items: &[Value],
        users: &[Value],
    ) -> usize {
        let mut fresh: Vec<(String, String, u64)> = keyed(items)
            .map(|(key, body)| (key, body, ITEM_TTL))
            .collect();
        let written: usize = fresh.iter().map(|(_, body, _)| body.len()).sum();
        if let Ok(body) = serde_json::to_string(window) {
            fresh.push((window_key(scope, qh), body, ttl));
        }
        let _ = self.cache.set_many_raw(&fresh).await;
        let missing: Vec<(String, String)> = keyed(users).collect();
        let _ = self.cache.set_many_raw_nx(&missing, ITEM_TTL).await;
        written
    }

    pub async fn failed(&self, scope: &str, qh: &str, now: i64) -> Option<i64> {
        let raw = self.cache.get_raw(&fail_key(scope, qh)).await.ok()??;
        let until: i64 = raw.parse().ok()?;
        (until > now).then_some(until - now)
    }

    pub async fn mark_failed(&self, scope: &str, qh: &str, now: i64) {
        let until = now + FAIL_TTL as i64;
        let _ = self
            .cache
            .set_many_raw(&[(fail_key(scope, qh), until.to_string(), FAIL_TTL)])
            .await;
    }
}

fn keyed(items: &[Value]) -> impl Iterator<Item = (String, String)> + '_ {
    items.iter().filter_map(|item| {
        let urn = urn_of(item)?;
        Some((item_key(urn), serde_json::to_string(item).ok()?))
    })
}

fn window_key(scope: &str, qh: &str) -> String {
    format!("{PREFIX}:w:{scope}:{qh}")
}

fn item_key(urn: &str) -> String {
    format!("{PREFIX}:i:{urn}")
}

fn fail_key(scope: &str, qh: &str) -> String {
    format!("{PREFIX}:f:{scope}:{qh}")
}
