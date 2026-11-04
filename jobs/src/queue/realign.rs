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

const STEP: i64 = 500;
const ROW_PAUSE: Duration = Duration::from_millis(20);
const REPEAT: Duration = Duration::from_secs(10 * 60);
const RETRY: Duration = Duration::from_secs(30);

pub(crate) struct LaneRealign {
    pool: PgPool,
    step: i64,
    row_pause: Duration,
    repeat: Duration,
    finished: AtomicBool,
}

impl LaneRealign {
    pub(crate) fn new(pool: PgPool) -> Arc<Self> {
        Arc::new(Self {
            pool,
            step: STEP,
            row_pause: ROW_PAUSE,
            repeat: REPEAT,
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
        loop {
            let moved = tokio::select! {
                _ = cancellation.cancelled() => return Ok(()),
                moved = self.pass() => moved,
            };
            self.finished.store(true, Ordering::Release);
            if moved > 0 {
                tracing::warn!(moved, "queued jobs moved to the lane their kind runs in");
                continue;
            }
            tokio::select! {
                _ = cancellation.cancelled() => return Ok(()),
                _ = tokio::time::sleep(self.repeat) => {}
            }
        }
    }

    async fn pass(&self) -> i64 {
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
                self.step
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
                    tokio::time::sleep(pause_after(self.row_pause, step.moved)).await;
                }
                Err(error) => {
                    tracing::warn!(%error, "queued jobs wait in a stale lane, the realignment retries");
                    tokio::time::sleep(RETRY).await;
                }
            }
        }
    }
}

fn pause_after(row_pause: Duration, moved: i64) -> Duration {
    row_pause.saturating_mul(u32::try_from(moved.max(0)).unwrap_or(u32::MAX))
}
