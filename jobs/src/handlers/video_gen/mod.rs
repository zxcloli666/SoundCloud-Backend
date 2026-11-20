pub mod director;
mod flow;
mod lanes;
pub mod opus;
mod parts;
mod prompts;
mod result;
mod state;
#[cfg(test)]
mod tests;

use std::future::Future;
use std::sync::Arc;

use chrono::Utc;
use sqlx::PgPool;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use url::Url;

use crate::config::VideoGenConfig;
use crate::handlers::video_accounts::Notify;
use crate::llm::ledger;
use crate::objects::{Blobs, Objects, ObjectsError};
use crate::queue::JobResult;

pub use self::director::Director;
pub use self::lanes::{Fleet, Lanes};
use self::lanes::{Nodes, STAGE_LANES};
use self::parts::Parts;
use self::state::{DIRECTOR_STAGES, LANE_STAGES};

pub const LADDER: &str = "video_gen";

pub trait Store: Blobs {
    fn get(&self, key: &str) -> impl Future<Output = Result<Option<Vec<u8>>, ObjectsError>> + Send;
}

impl Store for Objects {
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, ObjectsError> {
        self.get_bytes(key).await
    }
}

pub struct VideoGen<D, S, L, N> {
    pool: PgPool,
    storage_url: Url,
    config: VideoGenConfig,
    reset_day: u32,
    director: D,
    store: S,
    lanes: L,
    notify: N,
    parts: Parts,
}

impl<D: Director, S: Store, L: Nodes, N: Notify> VideoGen<D, S, L, N> {
    pub fn new(
        pool: PgPool,
        storage_url: Url,
        config: VideoGenConfig,
        director: D,
        store: S,
        lanes: L,
        notify: N,
    ) -> Self {
        let reset_day = config
            .director
            .as_ref()
            .map_or(1, |director| director.reset_day);
        Self {
            pool,
            storage_url,
            config,
            reset_day,
            director,
            store,
            lanes,
            notify,
            parts: Parts::default(),
        }
    }

    pub async fn sweep(self: Arc<Self>, cancellation: CancellationToken) -> anyhow::Result<()> {
        if self.config.in_flight <= 0 {
            info!("video generation is off: VIDEO_GEN_IN_FLIGHT is 0");
            cancellation.cancelled().await;
            return Ok(());
        }
        info!(
            in_flight = self.config.in_flight,
            month_usd = self.config.month_usd,
            reset_day = self.reset_day,
            "video generation started"
        );
        let mut ticker = tokio::time::interval(self.config.tick);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = cancellation.cancelled() => return Ok(()),
                _ = ticker.tick() => {}
            }
            if let Err(error) = self.dispatch().await {
                warn!(%error, "video generation tick failed; it resumes on the next tick");
            }
        }
    }

    pub(crate) async fn dispatch(&self) -> JobResult {
        let held = self.held_stages();
        state::hold_sent(&self.pool, &held).await?;
        state::requeue_unanswered(&self.pool, self.config.timeout).await?;
        let mut open = held.is_empty() && self.gate().await?;
        let (sent, active) = state::busy(&self.pool).await?;
        let free = self.config.in_flight - sent;
        if free <= 0 {
            return Ok(());
        }
        let room = self.config.in_flight * 2 - active;
        if open && room > 0 {
            state::enqueue(&self.pool, room).await?;
        }
        let stages: Vec<String> = LANE_STAGES
            .iter()
            .filter(|stage| !held.iter().any(|held| held == *stage))
            .chain(DIRECTOR_STAGES.iter().filter(|_| open))
            .map(|stage| (*stage).to_owned())
            .collect();
        for job in state::claim(&self.pool, free, &stages, self.config.lease).await? {
            if !open && DIRECTOR_STAGES.contains(&job.stage.as_str()) {
                continue;
            }
            let track = job.track.clone();
            match self.advance(job).await {
                Ok(flow::Flow::Moved) => {}
                Ok(flow::Flow::Halted) => open = false,
                Err(error) => warn!(track, %error, "video generation step failed"),
            }
        }
        Ok(())
    }

    fn held_stages(&self) -> Vec<String> {
        let held: Vec<String> = STAGE_LANES
            .iter()
            .filter(|(_, lane)| !self.lanes.serving(*lane))
            .map(|(stage, _)| (*stage).to_owned())
            .collect();
        if !held.is_empty() {
            debug!(
                ?held,
                "video generation waits for a node that serves its lanes"
            );
        }
        held
    }

    async fn gate(&self) -> JobResult<bool> {
        let paused = ledger::paused(&self.pool, LADDER).await?;
        match &paused {
            Some(pause) if !pause.due => return Ok(false),
            Some(pause) if pause.reason == ledger::API_DOWN => return Ok(true),
            _ => {}
        }
        if !self.affordable().await? {
            return Ok(false);
        }
        if paused.is_some() {
            ledger::resume(&self.pool, LADDER).await?;
        }
        Ok(true)
    }

    async fn affordable(&self) -> JobResult<bool> {
        let period = ledger::period(Utc::now(), self.reset_day);
        let spent = ledger::spent(&self.pool, LADDER, period).await?;
        if spent < self.config.month_usd {
            return Ok(true);
        }
        let text = format!(
            "Video generation is paused: the Opus budget of ${:.2} for the month from {period} is spent (${spent:.2}). It checks every {} min and resumes on day {} of the month.",
            self.config.month_usd,
            self.config.recheck.as_secs() / 60,
            self.reset_day
        );
        ledger::pause(
            &self.pool,
            &self.notify,
            LADDER,
            ledger::BUDGET,
            self.config.recheck,
            &text,
        )
        .await?;
        Ok(false)
    }
}
