use tracing::warn;

use super::director::{Director, Look};
use super::lanes::Nodes;
use super::state::{Gen, REJECTED_PREFIX, Stage, TEMP_PREFIX};
use super::{Store, VideoGen};
use crate::handlers::video_accounts::Notify;

impl<D: Director, S: Store, L: Nodes, N: Notify> VideoGen<D, S, L, N> {
    pub(super) async fn keep_reviewed(&self, job: &mut Gen, look: Look) {
        let reviewed: Vec<String> = match look {
            Look::Keyframe => job.state.frame.take().into_iter().collect(),
            Look::Shot => job
                .state
                .shot
                .take()
                .into_iter()
                .chain(std::mem::take(&mut job.state.stills))
                .collect(),
        };
        let kept = self.keep_rejected(&reviewed).await;
        job.state.temps.retain(|key| !moved(key, &kept));
        if let Some(rejection) = job.rejections.last_mut() {
            rejection.media.extend(kept);
        }
    }

    pub(super) async fn clear_temps(&self, job: &Gen) {
        match job.stage {
            Stage::Done => self.drop_temps(&job.state.temps).await,
            _ => {
                self.keep_rejected(&job.state.temps).await;
            }
        }
    }

    pub(super) async fn drop_temps(&self, keys: &[String]) {
        for key in keys {
            if let Err(error) = self.store.delete(key).await {
                warn!(key, %error, "a generation temp file was not deleted; the sweep removes it");
            }
        }
    }

    async fn keep_rejected(&self, keys: &[String]) -> Vec<String> {
        let mut kept = Vec::with_capacity(keys.len());
        for key in keys {
            let target = rejected_key(key);
            if let Err(error) = self.store.copy(key, &target).await {
                warn!(key, %error, "a rejected generation file was not kept; the sweep removes it");
                continue;
            }
            self.drop_temps(std::slice::from_ref(key)).await;
            kept.push(target);
        }
        kept
    }
}

fn rejected_key(key: &str) -> String {
    let name = key.strip_prefix(TEMP_PREFIX).unwrap_or(key);
    format!("{REJECTED_PREFIX}{name}")
}

fn moved(key: &str, kept: &[String]) -> bool {
    kept.contains(&rejected_key(key))
}
