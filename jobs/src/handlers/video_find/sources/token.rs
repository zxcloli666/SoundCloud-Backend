use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::Mutex;

use super::Failure;
use crate::handlers::video_accounts::{Fetch, SpotifyKey, access_token};

const REFRESH_MARGIN_MS: u64 = 60_000;

struct Cached {
    token: String,
    expires_ms: u64,
}

#[derive(Default)]
pub(super) struct Tokens {
    cached: Mutex<HashMap<String, Cached>>,
}

impl Tokens {
    pub(super) async fn get<F: Fetch>(
        &self,
        fetch: &F,
        key: &SpotifyKey,
        sp_dc: &str,
    ) -> Result<(String, bool), Failure> {
        let mut cached = self.cached.lock().await;
        let now = now_ms();
        if let Some(entry) = cached
            .get(sp_dc)
            .filter(|entry| entry.expires_ms > now + REFRESH_MARGIN_MS)
        {
            return Ok((entry.token.clone(), true));
        }
        let issued = access_token(fetch, key, sp_dc).await?;
        cached.retain(|_, entry| entry.expires_ms > now);
        cached.insert(
            sp_dc.to_owned(),
            Cached {
                token: issued.token.clone(),
                expires_ms: issued.expires_ms,
            },
        );
        Ok((issued.token, false))
    }

    pub(super) async fn forget(&self, sp_dc: &str) {
        self.cached.lock().await.remove(sp_dc);
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}
