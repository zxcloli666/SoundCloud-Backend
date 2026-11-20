mod frames;
pub mod haiku;
pub mod prompt;
mod queue;
mod result;
mod stored;
#[cfg(test)]
mod tests;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use backend_contracts::pipeline::{VideoLabel, VideoVlmRequest};
use backend_contracts::worker_contract::WorkerLane;
use chrono::Utc;
use sqlx::PgPool;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use url::Url;

use self::haiku::Vision;
use self::queue::{Claimed, Held, HeldFile, Known};
use crate::config::{VideoFindConfig, VideoVlmConfig};
use crate::handlers::video_accounts::Notify;
use crate::handlers::video_find::Pacing;
use crate::handlers::video_find::parts::Parts;
use crate::handlers::video_gen::Nodes;
use crate::llm::ledger;
use crate::objects::Blobs;
use crate::queue::{JobError, JobResult};

pub const LADDER: &str = "video_vlm";

const RETRY_PUBLISH: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Classify(WorkerLane),
    Frames,
    Cut(VideoLabel),
}

impl Step {
    const fn lane(self) -> WorkerLane {
        match self {
            Self::Classify(lane) => lane,
            Self::Frames | Self::Cut(_) => WorkerLane::VideoVlm,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Judge {
    Gpu,
    Haiku,
    Cpu,
}

impl Judge {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Gpu => "gpu",
            Self::Haiku => "haiku",
            Self::Cpu => "cpu",
        }
    }

    const fn step(self) -> Step {
        match self {
            Self::Gpu => Step::Classify(WorkerLane::VideoVlmGpu),
            Self::Haiku => Step::Frames,
            Self::Cpu => Step::Classify(WorkerLane::VideoVlm),
        }
    }
}

pub struct VideoVlm<B, L, V, N> {
    pool: PgPool,
    storage_url: Url,
    config: VideoVlmConfig,
    pacing: Pacing,
    blobs: B,
    parts: Parts,
    lanes: L,
    haiku: Option<V>,
    notify: N,
    reset_day: u32,
    path: Mutex<Option<Judge>>,
}

impl<B: Blobs + 'static, L: Nodes, V: Vision, N: Notify> VideoVlm<B, L, V, N> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool: PgPool,
        storage_url: Url,
        config: VideoVlmConfig,
        find: &VideoFindConfig,
        blobs: B,
        lanes: L,
        haiku: Option<V>,
        notify: N,
    ) -> Self {
        let reset_day = config.haiku.as_ref().map_or(1, |ladder| ladder.reset_day);
        Self {
            pool,
            storage_url,
            config,
            pacing: Pacing::of(find),
            blobs,
            parts: Parts::default(),
            lanes,
            haiku,
            notify,
            reset_day,
            path: Mutex::new(None),
        }
    }

    pub async fn sweep(self: Arc<Self>, cancellation: CancellationToken) -> anyhow::Result<()> {
        if self.config.in_flight <= 0 {
            info!("video VLM sweep is off: VIDEO_VLM_IN_FLIGHT is 0");
            cancellation.cancelled().await;
            return Ok(());
        }
        info!(
            in_flight = self.config.in_flight,
            tick_s = self.config.tick.as_secs(),
            haiku = self.haiku.is_some(),
            month_usd = self.config.month_usd,
            gpu_seen_s = self.config.gpu_seen.as_secs(),
            gpu_timeout_s = self.config.gpu_timeout.as_secs(),
            "video VLM sweep started"
        );
        let mut ticker = tokio::time::interval(self.config.tick);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = cancellation.cancelled() => return Ok(()),
                _ = ticker.tick() => {}
            }
            if let Err(error) = self.dispatch(&cancellation).await {
                warn!(%error, "video VLM dispatch failed; it resumes on the next tick");
            }
        }
    }

    pub(crate) async fn dispatch(&self, cancellation: &CancellationToken) -> JobResult {
        queue::requeue_unanswered(&self.pool, self.config.timeout, self.config.gpu_timeout).await?;
        self.verify_stored().await?;
        let free = self.config.in_flight - queue::busy(&self.pool).await?;
        if free <= 0 {
            return Ok(());
        }
        let claimed = queue::claim(&self.pool, free).await?;
        if claimed.is_empty() {
            return Ok(());
        }
        debug!(claimed = claimed.len(), "video VLM requests claimed");
        let judge = self.judge().await?;
        self.note(judge);
        for claim in claimed {
            if cancellation.is_cancelled() {
                self.release(&claim, Duration::ZERO, "shutdown").await;
                continue;
            }
            let judge = match (judge, claim.gpu_failed()) {
                (Judge::Gpu, true) => self.spare().await?,
                _ => judge,
            };
            self.send(&claim, judge).await;
        }
        Ok(())
    }

    async fn judge(&self) -> JobResult<Judge> {
        match self.lanes.serving(WorkerLane::VideoVlmGpu) {
            true => Ok(Judge::Gpu),
            false => self.spare().await,
        }
    }

    async fn spare(&self) -> JobResult<Judge> {
        match self.haiku_open().await? {
            true => Ok(Judge::Haiku),
            false => Ok(Judge::Cpu),
        }
    }

    fn local(&self, claim: &Claimed) -> Judge {
        match self.lanes.serving(WorkerLane::VideoVlmGpu) && !claim.gpu_failed() {
            true => Judge::Gpu,
            false => Judge::Cpu,
        }
    }

    fn note(&self, judge: Judge) {
        let mut last = self.path.lock().unwrap_or_else(PoisonError::into_inner);
        if *last != Some(judge) {
            info!(
                path = judge.as_str(),
                was = last.map_or("none", Judge::as_str),
                "video VLM checks switched path"
            );
            *last = Some(judge);
        }
    }

    async fn send(&self, claim: &Claimed, judge: Judge) {
        let Some(held) = &claim.held else {
            self.drop_unreadable(claim).await;
            return;
        };
        let step = match &held.haiku {
            Some(known) if known.label != VideoLabel::Static && held.file != HeldFile::Raw => {
                self.keep_known(claim, held, known).await;
                return;
            }
            Some(known) if known.label != VideoLabel::Static => Step::Cut(known.label),
            _ => judge.step(),
        };
        if let Err(error) = self.publish(claim, held, step).await {
            warn!(track = %claim.sc_track_id, %error, "video VLM request was not published");
            self.release(claim, RETRY_PUBLISH, "publish_failed").await;
            return;
        }
        if step.lane() != WorkerLane::VideoVlmGpu {
            return;
        }
        if let Err(error) = queue::sent_gpu(&self.pool, &claim.sc_track_id, claim.attempt).await {
            warn!(track = %claim.sc_track_id, %error, "GPU VLM send was not marked; it waits for the common timeout");
        }
    }

    async fn keep_known(&self, claim: &Claimed, held: &Held, known: &Known) {
        let (track, attempt) = (claim.sc_track_id.as_str(), claim.attempt);
        let kept = match held.file {
            HeldFile::Stored => self.keep_stored(track, attempt, held, known).await,
            _ => self.keep_cut(track, attempt, held, known).await,
        };
        if let Err(error) = kept {
            warn!(track, %error, "VLM verdict was not stored");
            self.release(claim, RETRY_PUBLISH, "store_failed").await;
        }
    }

    async fn publish(&self, claim: &Claimed, held: &Held, step: Step) -> JobResult {
        let request = self.request(claim, held, step)?;
        let payload = serde_json::to_value(&request).map_err(JobError::permanent)?;
        self.lanes
            .publish(
                step.lane().spec().filter_subject,
                payload,
                claim.message_id(held),
            )
            .await
            .map_err(JobError::retryable)
    }

    fn request(&self, claim: &Claimed, held: &Held, step: Step) -> JobResult<VideoVlmRequest> {
        let url = self
            .storage_url
            .join(&held.s3_key)
            .map_err(JobError::permanent)?;
        Ok(VideoVlmRequest {
            sc_track_id: claim.sc_track_id.clone(),
            attempt: i64::from(claim.attempt),
            video_url: url.to_string(),
            duration_ms: claim.duration_ms.filter(|duration| *duration > 0),
            offset_ms: Some(held.offset_ms),
            pad_ms: matches!(step, Step::Cut(_))
                .then_some(held.pad_ms)
                .filter(|pad| *pad > 0),
            frames: (step == Step::Frames).then_some(true),
            label: match step {
                Step::Cut(label) => Some(label),
                _ => None,
            },
            prompt: matches!(step, Step::Classify(_))
                .then(|| prompt::local(&prompt::Features::of(held))),
        })
    }

    async fn haiku_open(&self) -> JobResult<bool> {
        if self.haiku.is_none() {
            return Ok(false);
        }
        let paused = ledger::paused(&self.pool, LADDER).await?;
        if paused.as_ref().is_some_and(|pause| !pause.due) {
            return Ok(false);
        }
        let period = ledger::period(Utc::now(), self.reset_day);
        let spent = ledger::spent(&self.pool, LADDER, period).await?;
        if spent < self.config.month_usd {
            if paused.is_some() {
                ledger::resume(&self.pool, LADDER).await?;
            }
            return Ok(true);
        }
        let text = format!(
            "Video VLM checks moved to the local model: the Haiku budget of ${:.2} for the month from {period} is spent (${spent:.2}). Haiku comes back by itself on day {} of the month.",
            self.config.month_usd, self.reset_day
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

    async fn release(&self, claim: &Claimed, wait: Duration, why: &str) {
        queue::release(&self.pool, &claim.sc_track_id, claim.attempt, wait, why).await;
    }

    async fn drop_unreadable(&self, claim: &Claimed) {
        warn!(track = %claim.sc_track_id, "held video payload is unreadable; the candidate is dropped");
        let settled = queue::settle(
            &self.pool,
            &self.pacing,
            &claim.sc_track_id,
            claim.attempt,
            "none",
            self.pacing.none_recheck,
            Some("held_unreadable"),
        )
        .await;
        if let Err(error) = settled {
            warn!(track = %claim.sc_track_id, %error, "unreadable held video was not settled");
        }
    }
}
