use std::convert::Infallible;
use std::time::Duration;

use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::Retainer;

const BATCH: i32 = 500;
const PACE: Duration = Duration::from_secs(2);
const LEGACY_PACE: Duration = Duration::from_secs(60);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub struct Sweep {
    pub state: String,
    pub sent: i32,
    pub skipped: i32,
    pub gone: i32,
    pub queued: i64,
}

impl Sweep {
    pub fn work(&self) -> i32 {
        self.sent + self.skipped + self.gone
    }

    fn full(&self, batch: i32) -> bool {
        self.work() >= batch && self.sent + self.gone > 0
    }
}

pub async fn sweep_once(pool: &PgPool, batch: i32) -> Result<Sweep, sqlx::Error> {
    sqlx::query_file_as!(Sweep, "queries/sweep.sql", batch)
        .fetch_one(pool)
        .await
}

pub struct Sweeper {
    pool: PgPool,
    batch: i32,
    pace: Duration,
}

impl Sweeper {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            batch: BATCH,
            pace: PACE,
        }
    }

    pub fn with_pace(self, pace: Duration) -> Self {
        Self { pace, ..self }
    }

    pub async fn run(self, stop: CancellationToken) -> Result<(), Infallible> {
        let retainer = Retainer::new(self.pool.clone());
        let (swept, retained) = tokio::join!(self.sweep(stop.clone()), retainer.run(stop));
        swept.and(retained)
    }

    async fn sweep(self, stop: CancellationToken) -> Result<(), Infallible> {
        let mut backoff = self.pace;
        loop {
            let wait = match sweep_once(&self.pool, self.batch).await {
                Ok(sweep) => {
                    backoff = self.pace;
                    record(&sweep);
                    if sweep.state == "legacy" {
                        LEGACY_PACE
                    } else if sweep.full(self.batch) {
                        Duration::ZERO
                    } else {
                        self.pace
                    }
                }
                Err(error) => {
                    warn!(%error, "scd resend sweep failed");
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                    backoff
                }
            };
            tokio::select! {
                () = stop.cancelled() => return Ok(()),
                () = tokio::time::sleep(wait) => {}
            }
        }
    }
}

fn record(sweep: &Sweep) {
    if sweep.state != "ran" {
        return;
    }
    metrics::gauge!("scd_repl_resend_queued").set(sweep.queued as f64);
    metrics::counter!("scd_repl_resend_sent_total").increment(sweep.sent as u64);
    metrics::counter!("scd_repl_resend_skipped_locked_total").increment(sweep.skipped as u64);
    metrics::counter!("scd_repl_resend_gone_total").increment(sweep.gone as u64);
}
