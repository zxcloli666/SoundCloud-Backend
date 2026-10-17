use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use wreq::Client;

use super::anon::AnonClient;
use super::cookies::{CookieStreamResult, CookiesClient, PREVIEWS_ONLY};
use super::quality;
use super::restricted::{RestrictedSource, Transcoding};
use crate::config::parse_cookie_value;

type BoxErr = Box<dyn std::error::Error + Send + Sync>;

const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(30);
const REJECTED_COOLDOWN: Duration = Duration::from_secs(600);
const UQ_MEMORY: Duration = Duration::from_secs(6 * 60 * 60);

struct PoolEntry {
    client: CookiesClient,
    cooldown_until: Mutex<Option<tokio::time::Instant>>,
}

pub struct CookiesPool {
    entries: Vec<PoolEntry>,
    cursor: AtomicUsize,
    uq_seen_at: AtomicU64,
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
            uq_seen_at: AtomicU64::new(0),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn serves_uq(&self) -> bool {
        recently(self.uq_seen_at.load(Ordering::Relaxed), now_secs())
    }

    fn note_quality(&self, seen: &str) {
        if seen == quality::UQ {
            self.uq_seen_at.store(now_secs(), Ordering::Relaxed);
        }
    }

    async fn try_rotate<'a, F, Fut, T>(&'a self, authed: bool, mut op: F) -> Result<T, BoxErr>
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
                    if msg == PREVIEWS_ONLY {
                        debug!("[cookies-pool] client #{idx} sees previews only");
                        last_err = Some(e);
                        continue;
                    }
                    let cooldown = if is_rate_limited(&msg) {
                        warn!("[cookies-pool] client #{idx} rate-limited: {msg}");
                        RATE_LIMIT_COOLDOWN
                    } else if authed && is_rejected(&msg) {
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
        let result = self
            .try_rotate(false, |client| async move {
                client.get_stream(track_urn, hq_only).await
            })
            .await?;
        if let Some(stream) = &result {
            self.note_quality(stream.quality);
        }
        Ok(result)
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
        let meta = self
            .try_rotate(true, |client| async move {
                let (tcs, auth, cid) = client.fetch_track_meta(track_urn).await?;
                Ok((tcs, auth, cid, client.cookie_auth_headers()))
            })
            .await?;
        let full = meta.0.iter().filter(|t| !t.snipped.unwrap_or(false));
        if let Some(best) = full.max_by_key(|t| quality::rank(quality::of_transcoding(t))) {
            self.note_quality(quality::of_transcoding(best));
        }
        Ok(meta)
    }

    pub(crate) async fn resolve_restricted(
        self: &Arc<Self>,
        track_urn: &str,
        uq_first: bool,
        at_least: &'static str,
    ) -> Result<Option<RestrictedSource>, BoxErr> {
        let source = self
            .try_rotate(true, |client| async move {
                client
                    .resolve_restricted(track_urn, uq_first, at_least)
                    .await
            })
            .await?;
        if let Some(found) = &source {
            self.note_quality(found.quality);
        }
        Ok(source)
    }

    pub fn log_summary(&self) {
        info!(
            "[cookies-pool] initialized with {} client(s)",
            self.entries.len()
        );
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn recently(seen_at: u64, now: u64) -> bool {
    seen_at > 0 && now.saturating_sub(seen_at) < UQ_MEMORY.as_secs()
}

fn is_rate_limited(msg: &str) -> bool {
    msg.ends_with("status 429") || msg.to_ascii_lowercase().contains("too many requests")
}

fn is_rejected(msg: &str) -> bool {
    msg.ends_with("status 401")
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{
        BoxErr, CookiesPool, PREVIEWS_ONLY, UQ_MEMORY, is_rate_limited, is_rejected, recently,
    };
    use crate::stream::quality;

    #[test]
    fn detects_429_variants() {
        assert!(is_rate_limited("status 429"));
        assert!(is_rate_limited("HTTP 429 Too Many Requests"));
        assert!(is_rate_limited("relay status 429"));
        assert!(!is_rate_limited("status 404"));
        assert!(!is_rate_limited("status 502"));
        assert!(!is_rate_limited("connection reset"));
        assert!(!is_rate_limited("no transcodings for tracks:4290"));
    }

    #[test]
    fn an_expired_account_is_told_apart_from_a_missing_track() {
        assert!(is_rejected("status 401"));
        assert!(is_rejected("relay status 401"));
        assert!(!is_rejected("status 403"));
        assert!(!is_rejected("status 404"));
        assert!(!is_rejected("cookies: no transcodings"));
        assert!(!is_rejected(PREVIEWS_ONLY));
    }

    #[tokio::test]
    async fn previews_only_moves_on_without_a_cooldown() {
        let cookies = [
            "oauth_token=first".to_string(),
            "oauth_token=second".to_string(),
        ];
        let pool = CookiesPool::new(wreq::Client::new(), "", &cookies);
        let calls = AtomicUsize::new(0);

        let result: Result<(), BoxErr> = pool
            .try_rotate(true, |_| {
                calls.fetch_add(1, Ordering::Relaxed);
                async { Err(PREVIEWS_ONLY.into()) }
            })
            .await;

        assert_eq!(result.unwrap_err().to_string(), PREVIEWS_ONLY);
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        for entry in &pool.entries {
            assert!(entry.cooldown_until.lock().await.is_none());
        }
    }

    #[tokio::test]
    async fn the_cursor_lands_on_the_first_account_with_full_transcodings() {
        let cookies = [
            "oauth_token=first".to_string(),
            "oauth_token=second".to_string(),
            "oauth_token=third".to_string(),
        ];
        let pool = CookiesPool::new(wreq::Client::new(), "", &cookies);
        let calls = AtomicUsize::new(0);

        let result: Result<usize, BoxErr> = pool
            .try_rotate(true, |_| {
                let call = calls.fetch_add(1, Ordering::Relaxed);
                async move {
                    if call < 2 {
                        Err(PREVIEWS_ONLY.into())
                    } else {
                        Ok(call)
                    }
                }
            })
            .await;

        assert_eq!(result.unwrap(), 2);
        assert_eq!(pool.cursor.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn uq_is_tried_only_after_an_account_showed_it_lately() {
        let pool = CookiesPool::new(wreq::Client::new(), "", &["oauth_token=a".to_string()]);
        assert!(!pool.serves_uq());

        pool.note_quality(quality::HQ);
        assert!(!pool.serves_uq());

        pool.note_quality(quality::UQ);
        assert!(pool.serves_uq());
    }

    #[test]
    fn an_old_sighting_of_uq_wears_off() {
        let memory = UQ_MEMORY.as_secs();
        assert!(!recently(0, 1_000_000));
        assert!(recently(1_000_000, 1_000_000 + memory - 1));
        assert!(!recently(1_000_000, 1_000_000 + memory));
    }
}
