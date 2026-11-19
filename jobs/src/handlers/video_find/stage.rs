use std::path::PathBuf;

use backend_contracts::admin_video::VideoSource;
use backend_contracts::pipeline::{VideoCandidate, VideoCandidateKind};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::VideoFind;
use super::leads::{Leased, pause_for};
use super::queue::Claimed;
use super::sources::{Failure, Lead};
use super::ytdlp::{Exit, Tube};
use crate::handlers::video_accounts::Fetch;
use crate::objects::{Blobs, ObjectsError};
use crate::queue::{JobError, JobResult};

pub(super) const TEMP_PREFIX: &str = "video-tmp/";

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Staged {
    pub(super) candidates: Vec<StagedCandidate>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct StagedCandidate {
    pub(super) index: u32,
    pub(super) source: VideoSource,
    pub(super) kind: VideoCandidateKind,
    pub(super) source_ref: Option<String>,
    pub(super) key: String,
}

impl Staged {
    pub(super) fn keys(&self) -> Vec<String> {
        self.candidates
            .iter()
            .map(|candidate| candidate.key.clone())
            .collect()
    }

    pub(super) fn candidate(&self, index: u32) -> Option<&StagedCandidate> {
        self.candidates
            .iter()
            .find(|candidate| candidate.index == index)
    }
}

pub(super) fn temp_key(track: &str, attempt: i32, index: u32) -> String {
    format!("{TEMP_PREFIX}{track}-{attempt}-{index}.mp4")
}

enum Bring {
    Source(Failure),
    Store(ObjectsError),
}

impl From<Failure> for Bring {
    fn from(failure: Failure) -> Self {
        Self::Source(failure)
    }
}

impl From<ObjectsError> for Bring {
    fn from(error: ObjectsError) -> Self {
        Self::Store(error)
    }
}

impl<F: Fetch + 'static, B: Blobs + 'static, T: Tube + 'static> VideoFind<F, B, T> {
    pub(super) async fn bring_all(
        &self,
        claim: &Claimed,
        leads: Vec<Lead>,
        youtube: &Leased,
    ) -> JobResult<Staged> {
        let mut staged = Staged::default();
        let mut youtube_open = true;
        for lead in leads {
            let is_youtube = lead.source == VideoSource::Youtube;
            if is_youtube && !youtube_open {
                continue;
            }
            let index = u32::try_from(staged.candidates.len()).unwrap_or(u32::MAX);
            let key = temp_key(&claim.sc_track_id, claim.attempt, index);
            match self.bring(&lead, &key, &youtube.cookies).await {
                Ok(()) => staged.candidates.push(StagedCandidate {
                    index,
                    source: lead.source,
                    kind: lead.kind,
                    source_ref: lead.source_ref.clone(),
                    key,
                }),
                Err(Bring::Store(error)) => {
                    self.drop_temps(&staged.keys()).await;
                    return Err(JobError::retryable(error));
                }
                Err(Bring::Source(failure)) if is_youtube => {
                    youtube_open = self.youtube_failed(&youtube.name, failure).await;
                }
                Err(Bring::Source(failure)) => {
                    debug!(url = %lead.url, ?failure, "video candidate was not fetched");
                }
            }
            if is_youtube {
                tokio::time::sleep(self.config.youtube_pause).await;
            }
        }
        Ok(staged)
    }

    async fn youtube_failed(&self, account: &str, failure: Failure) -> bool {
        match failure {
            Failure::Paused(error, retry_after) => {
                let pause = pause_for(self.config.account_pause, retry_after);
                self.account_out(account, &error, Some(pause)).await;
                false
            }
            Failure::Rejected(error) => {
                self.account_out(account, &error, None).await;
                false
            }
            Failure::Unavailable(error) => {
                warn!(account, %error, "a youtube candidate was not fetched");
                true
            }
        }
    }

    async fn bring(&self, lead: &Lead, key: &str, cookies: &str) -> Result<(), Bring> {
        match lead.kind {
            VideoCandidateKind::Canvas => {
                let bytes = self.sources.file(&lead.url).await?;
                self.blobs.put_bytes(key, bytes.to_vec()).await?;
                Ok(())
            }
            VideoCandidateKind::Video => {
                let folder = self.config.work_dir.join(key.replace('/', "_"));
                let outcome = self.download_and_put(lead, key, &folder, cookies).await;
                let _ = tokio::fs::remove_dir_all(&folder).await;
                outcome
            }
        }
    }

    async fn download_and_put(
        &self,
        lead: &Lead,
        key: &str,
        folder: &PathBuf,
        cookies: &str,
    ) -> Result<(), Bring> {
        tokio::fs::create_dir_all(folder)
            .await
            .map_err(|error| Failure::Unavailable(format!("work dir: {error}")))?;
        let youtube = lead.source == VideoSource::Youtube;
        let exit = if lead.source == VideoSource::Yandex {
            Exit::Russia
        } else {
            Exit::Anywhere
        };
        let path = self
            .tube
            .download(
                &lead.url,
                &folder.join("clip"),
                youtube.then_some(cookies),
                exit,
            )
            .await?;
        self.blobs.put_file(key, &path).await?;
        Ok(())
    }

    pub(super) fn candidate_list(&self, staged: &Staged) -> Result<Vec<VideoCandidate>, JobError> {
        staged
            .candidates
            .iter()
            .map(|candidate| {
                let url = self
                    .storage_url
                    .join(&candidate.key)
                    .map_err(JobError::permanent)?;
                Ok(VideoCandidate {
                    index: candidate.index,
                    source: candidate.source,
                    kind: candidate.kind,
                    url: url.to_string(),
                    source_ref: candidate.source_ref.clone(),
                })
            })
            .collect()
    }

    pub(super) async fn drop_temps(&self, keys: &[String]) {
        for key in keys {
            if let Err(error) = self.blobs.delete(key).await {
                warn!(key, %error, "a video temp file was not deleted; the sweep removes it");
            }
        }
    }
}
