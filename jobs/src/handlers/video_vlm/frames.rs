use backend_contracts::pipeline::VideoLabel;
use backend_contracts::video_gen::VideoFileSlice;
use chrono::Utc;
use serde_json::Map;
use tracing::{debug, info, warn};

use super::haiku::{Judged, Vision};
use super::prompt::Features;
use super::queue::{self, Known};
use super::result::Frames;
use super::{LADDER, VideoVlm};
use crate::handlers::video_accounts::Notify;
use crate::handlers::video_gen::Nodes;
use crate::llm::ledger;
use crate::objects::Blobs;
use crate::queue::{JobError, JobResult};

impl<B: Blobs + 'static, L: Nodes, V: Vision, N: Notify> VideoVlm<B, L, V, N> {
    pub(super) async fn framed(&self, track: &str, attempt: i32, frames: &Frames) -> JobResult {
        let joined = self
            .parts
            .add(track, i64::from(attempt), &frames.clip, &frames.part)
            .map_err(JobError::permanent)?;
        let Some(blob) = joined else {
            return Ok(());
        };
        let Some(held) = queue::held(&self.pool, track, attempt).await? else {
            debug!(track, attempt, "stale VLM frames ignored");
            return Ok(());
        };
        let pictures = cut(&blob, &frames.files).map_err(JobError::permanent)?;
        match self.ask(track, &pictures, &Features::of(&held)).await? {
            Some(judged) if judged.label == VideoLabel::Static => {
                self.settle_static(track, attempt, &held, &judged.model)
                    .await
            }
            Some(judged) => {
                let known = Known {
                    label: judged.label,
                    model: judged.model,
                    scores: Map::new(),
                };
                self.next(track, attempt, Some(&known)).await
            }
            None => self.next(track, attempt, None).await,
        }
    }

    pub(super) async fn answered(&self, track: &str, attempt: i32, answer: &Known) -> JobResult {
        let Some(held) = queue::held(&self.pool, track, attempt).await? else {
            debug!(track, attempt, "stale VLM answer ignored");
            return Ok(());
        };
        let known = Known {
            label: Features::of(&held).kind(answer.label),
            ..answer.clone()
        };
        info!(
            track,
            answer = answer.label.as_str(),
            label = known.label.as_str(),
            model = known.model,
            "local VLM judged a video"
        );
        self.next(track, attempt, Some(&known)).await
    }

    async fn ask(
        &self,
        track: &str,
        pictures: &[Vec<u8>],
        features: &Features,
    ) -> JobResult<Option<Judged>> {
        let Some(haiku) = &self.haiku else {
            return Ok(None);
        };
        if !self.haiku_open().await? {
            return Ok(None);
        }
        let answer = haiku.judge(pictures, features).await;
        let period = ledger::period(Utc::now(), self.reset_day);
        ledger::record(&self.pool, LADDER, period, track, &answer.spend).await?;
        match answer.result {
            Ok(judged) => {
                info!(
                    track,
                    label = judged.label.as_str(),
                    model = judged.model,
                    reason = judged.reason,
                    "Haiku judged a video"
                );
                Ok(Some(judged))
            }
            Err(error) => {
                warn!(track, %error, "Haiku gave no label; the local model checks the video");
                Ok(None)
            }
        }
    }

    async fn next(&self, track: &str, attempt: i32, known: Option<&Known>) -> JobResult {
        let Some(claim) = queue::next(&self.pool, track, attempt, known).await? else {
            debug!(track, attempt, "stale VLM frames ignored");
            return Ok(());
        };
        self.send(&claim, self.local(&claim)).await;
        Ok(())
    }
}

fn cut(blob: &[u8], files: &[VideoFileSlice]) -> anyhow::Result<Vec<Vec<u8>>> {
    files
        .iter()
        .map(|file| {
            let start = usize::try_from(file.offset)?;
            let end = start + usize::try_from(file.bytes)?;
            blob.get(start..end)
                .map(<[u8]>::to_vec)
                .ok_or_else(|| anyhow::anyhow!("VLM frame {} is outside the file", file.name))
        })
        .collect()
}
