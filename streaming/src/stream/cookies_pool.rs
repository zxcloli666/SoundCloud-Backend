use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::Mutex;
use tracing::{info, warn};
use wreq::Client;

use super::anon::AnonClient;
use super::cookies::{CookieStreamResult, CookiesClient};
use super::restricted::{RestrictedSource, Transcoding};
use crate::config::parse_cookie_value;

type BoxErr = Box<dyn std::error::Error + Send + Sync>;

const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(30);
const REJECTED_COOLDOWN: Duration = Duration::from_secs(600);

struct PoolEntry {
    client: CookiesClient,
    cooldown_until: Mutex<Option<tokio::time::Instant>>,
}

pub struct CookiesPool {
    entries: Vec<PoolEntry>,
    cursor: AtomicUsize,
}

impl CookiesPool {
    pub fn new(http: Client, proxy_url: &str, cookies_list: &[String]) -> Self {
        let entries = cookies_list
            .iter()
            .filter_map(|raw| {
                let token = parse_cookie_value(raw, "oauth_token")?;
                let client = CookiesClient::new(
                    http.clone(),
                    proxy_url.to_string(),
                    raw.clone(),
                    token,
                    AnonClient::new(http.clone(), proxy_url.to_string()),
                );
                Some(PoolEntry {
                    client,
                    cooldown_until: Mutex::new(None),
                })
            })
            .collect::<Vec<_>>();
        Self {
            entries,
            cursor: AtomicUsize::new(0),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    async fn try_rotate<'a, F, Fut, T>(&'a self, mut op: F) -> Result<T, BoxErr>
    where
        F: FnMut(&'a CookiesClient) -> Fut,
        Fut: std::future::Future<Output = Result<T, BoxErr>>,
    {
        let n = self.entries.len();
        if n == 0 {
            return Err("cookies pool empty".into());
        }
        let start = self.cursor.load(Ordering::Relaxed) % n;
        let mut last_err: Option<BoxErr> = None;
        let now = tokio::time::Instant::now();

        for off in 0..n {
            let idx = (start + off) % n;
            let entry = &self.entries[idx];

            if let Some(until) = *entry.cooldown_until.lock().await
                && until > now
            {
                continue;
            }

            match op(&entry.client).await {
                Ok(v) => {
                    self.cursor.store(idx, Ordering::Relaxed);
                    *entry.cooldown_until.lock().await = None;
                    return Ok(v);
                }
                Err(e) => {
                    let msg = e.to_string();
                    let cooldown = if is_rate_limited(&msg) {
                        warn!("[cookies-pool] client #{idx} rate-limited: {msg}");
                        RATE_LIMIT_COOLDOWN
                    } else if is_rejected(&msg) {
                        warn!("[cookies-pool] client #{idx} rejected, cookies look expired: {msg}");
                        REJECTED_COOLDOWN
                    } else {
                        return Err(e);
                    };
                    *entry.cooldown_until.lock().await = Some(now + cooldown);
                    last_err = Some(e);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| "all cookies clients cooling down".into()))
    }

    pub async fn get_stream(
        self: &Arc<Self>,
        track_urn: &str,
        hq_only: bool,
    ) -> Result<Option<CookieStreamResult>, BoxErr> {
        self.try_rotate(|client| async move { client.get_stream(track_urn, hq_only).await })
            .await
    }

    pub async fn fetch_track_meta(
        self: &Arc<Self>,
        track_urn: &str,
    ) -> Result<
        (
            Vec<Transcoding>,
            Option<String>,
            String,
            HashMap<String, String>,
        ),
        BoxErr,
    > {
        self.try_rotate(|client| async move {
            let (tcs, auth, cid) = client.fetch_track_meta(track_urn).await?;
            Ok((tcs, auth, cid, client.cookie_auth_headers()))
        })
        .await
    }

    pub(crate) async fn resolve_restricted(
        self: &Arc<Self>,
        track_urn: &str,
        hq_first: bool,
    ) -> Result<Option<RestrictedSource>, BoxErr> {
        self.try_rotate(
            |client| async move { client.resolve_restricted(track_urn, hq_first).await },
        )
        .await
    }

    pub fn log_summary(&self) {
        info!(
            "[cookies-pool] initialized with {} client(s)",
            self.entries.len()
        );
    }
}

fn is_rate_limited(msg: &str) -> bool {
    msg.contains("429") || msg.to_ascii_lowercase().contains("too many requests")
}

fn is_rejected(msg: &str) -> bool {
    msg.ends_with("status 401")
}

#[cfg(test)]
mod tests {
    use super::{is_rate_limited, is_rejected};

    #[test]
    fn detects_429_variants() {
        assert!(is_rate_limited("status 429"));
        assert!(is_rate_limited("HTTP 429 Too Many Requests"));
        assert!(is_rate_limited("relay status 429"));
        assert!(!is_rate_limited("status 404"));
        assert!(!is_rate_limited("status 502"));
        assert!(!is_rate_limited("connection reset"));
    }

    #[test]
    fn an_expired_account_is_told_apart_from_a_missing_track() {
        assert!(is_rejected("status 401"));
        assert!(is_rejected("relay status 401"));
        assert!(!is_rejected("status 403"));
        assert!(!is_rejected("status 404"));
        assert!(!is_rejected("cookies: no transcodings"));
    }
}
