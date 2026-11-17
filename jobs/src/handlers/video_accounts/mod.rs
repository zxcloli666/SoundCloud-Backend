mod alert;
mod check;
mod rpc;
mod store;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use backend_contracts::admin_video::{
    ADMIN_VIDEO_QUEUE, ADMIN_VIDEO_SUBJECT, AdminVideoReply, AdminVideoRequest, VideoSource,
};
use sqlx::PgPool;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::bus::Bus;

pub(crate) use self::alert::{Notify, Telegram};
pub(crate) use self::check::{Checker, Fetch, Proxied, SpotifyKey, Verdict};

const TICK: Duration = Duration::from_secs(3_600);
const CONCURRENCY: usize = 4;

pub struct VideoAccounts<F, N> {
    pool: PgPool,
    checker: Checker<F>,
    notify: N,
    check_every: Duration,
}

impl<F: Fetch + 'static, N: Notify + 'static> VideoAccounts<F, N> {
    pub fn new(pool: PgPool, checker: Checker<F>, notify: N, check_every: Duration) -> Self {
        Self {
            pool,
            checker,
            notify,
            check_every,
        }
    }

    pub async fn serve(
        self: Arc<Self>,
        bus: Bus,
        cancellation: CancellationToken,
    ) -> anyhow::Result<()> {
        bus.serve(
            ADMIN_VIDEO_SUBJECT,
            ADMIN_VIDEO_QUEUE,
            CONCURRENCY,
            cancellation,
            move |request: AdminVideoRequest| {
                let accounts = self.clone();
                async move { accounts.reply(request).await }
            },
        )
        .await
    }

    pub async fn daily(self: Arc<Self>, cancellation: CancellationToken) -> anyhow::Result<()> {
        info!(
            every_s = self.check_every.as_secs(),
            "video account checks started"
        );
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = cancellation.cancelled() => return Ok(()),
                _ = ticker.tick() => {}
            }
            if let Err(error) = self.check_due(&cancellation).await {
                warn!(%error, "video account checks failed; they resume on the next tick");
            }
        }
    }

    async fn check_due(&self, cancellation: &CancellationToken) -> Result<(), sqlx::Error> {
        for name in store::due(&self.pool, self.check_every).await? {
            if cancellation.is_cancelled() {
                break;
            }
            self.check_account(&name).await?;
        }
        self.send_dead_alerts().await
    }

    async fn check_account(&self, name: &str) -> Result<Option<Verdict>, sqlx::Error> {
        let Some((source, secret)) = store::secret_of(&self.pool, name).await? else {
            return Ok(None);
        };
        let Some(source) = VideoSource::parse(&source) else {
            return Ok(None);
        };
        let verdict = self.checker.check(source, &secret).await;
        match &verdict {
            Verdict::Valid => store::record(&self.pool, name, Some("ok"), None).await?,
            Verdict::Rejected(error) => {
                store::record(&self.pool, name, Some("dead"), Some(error)).await?
            }
            Verdict::Unavailable(error) => {
                store::record(&self.pool, name, None, Some(error)).await?
            }
        }
        self.send_dead_alerts().await?;
        Ok(Some(verdict))
    }

    async fn send_dead_alerts(&self) -> Result<(), sqlx::Error> {
        for dead in store::claim_alerts(&self.pool).await? {
            let text = alert::dead_text(&dead.name, &dead.source, dead.last_error.as_deref());
            if let Err(error) = self.notify.send(&text).await {
                warn!(account = %dead.name, %error, "the dead video account alert was not sent; it is retried");
                store::release_alert(&self.pool, &dead.name).await?;
            }
        }
        Ok(())
    }

    async fn reply(&self, request: AdminVideoRequest) -> anyhow::Result<AdminVideoReply> {
        rpc::handle(self, request.command).await
    }
}
