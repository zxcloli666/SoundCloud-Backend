use std::sync::Arc;
use std::time::SystemTime;

use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::VideoFind;
use super::stage::TEMP_PREFIX;
use super::ytdlp::Tube;
use crate::handlers::video_accounts::Fetch;
use crate::handlers::video_gen::REJECTED_PREFIX;
use crate::objects::Blobs;

impl<F: Fetch + 'static, B: Blobs + 'static, T: Tube + 'static> VideoFind<F, B, T> {
    pub async fn expire(self: Arc<Self>, cancellation: CancellationToken) -> anyhow::Result<()> {
        let mut ticker = tokio::time::interval(self.config.sweep_every);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = cancellation.cancelled() => return Ok(()),
                _ = ticker.tick() => {}
            }
            if let Err(error) = self.expire_once().await {
                warn!(%error, "video temp sweep failed; it resumes on the next tick");
            }
        }
    }

    pub(super) async fn expire_once(&self) -> anyhow::Result<usize> {
        let now = SystemTime::now();
        let mut removed = 0;
        for listed in self.blobs.list(TEMP_PREFIX).await? {
            let ttl = match listed.key.starts_with(REJECTED_PREFIX) {
                true => self.config.rejected_ttl,
                false => self.config.temp_ttl,
            };
            if listed.modified < now - ttl {
                self.blobs.delete(&listed.key).await?;
                removed += 1;
            }
        }
        if removed > 0 {
            info!(removed, "expired video temp files deleted");
        }
        Ok(removed)
    }
}
