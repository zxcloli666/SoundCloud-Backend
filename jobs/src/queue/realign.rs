#[cfg(test)]
#[path = "realign_tests.rs"]
mod tests;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use backend_contracts::JobKind;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const BATCH: i64 = 1_000;
const PAUSE: Duration = Duration::from_secs(1);
const RETRY: Duration = Duration::from_secs(30);

pub(crate) struct LaneRealign {
    pool: PgPool,
    batch: i64,
    pause: Duration,
    finished: AtomicBool,
}

impl LaneRealign {
    pub(crate) fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self {
            pool,
            batch: BATCH,
            pause: PAUSE,
            finished: AtomicBool::new(false),
        })
    }

    pub(crate) fn finished(&self) -> bool {
        self.finished.load(Ordering::Acquire)
    }

    pub(crate) async fn run(
        self: Arc<Self>,
        cancellation: CancellationToken,
    ) -> anyhow::Result<()> {
        let moved = tokio::select! {
            _ = cancellation.cancelled() => return Ok(()),
            moved = self.realign() => moved,
        };
        self.finished.store(true, Ordering::Release);
        if moved > 0 {
            tracing::warn!(moved, "queued jobs moved to the lane their kind runs in");
        }
        cancellation.cancelled().await;
        Ok(())
    }

    async fn realign(&self) -> i64 {
        let (kinds, lanes): (Vec<String>, Vec<String>) = JobKind::ALL
            .iter()
            .map(|kind| (kind.as_str().to_owned(), kind.lane().as_str().to_owned()))
            .unzip();
        let mut after = Uuid::nil();
        let mut moved = 0;
        loop {
            let step = sqlx::query_file!(
                "queries/queue/realign_lanes.sql",
                &kinds,
                &lanes,
                after,
                self.batch
            )
            .fetch_one(&self.pool)
            .await;
            match step {
                Ok(step) => {
                    let Some(last) = step.last else {
                        return moved;
                    };
                    after = last;
                    moved += step.moved;
                    if step.moved > 0 {
                        tokio::time::sleep(self.pause).await;
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "queued jobs wait in a stale lane, the realignment retries");
                    tokio::time::sleep(RETRY).await;
                }
            }
        }
    }
}
