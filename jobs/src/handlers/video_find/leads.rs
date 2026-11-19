use std::collections::HashSet;
use std::time::Duration;

use backend_contracts::admin_video::VideoSource;
use backend_contracts::pipeline::VideoCandidateKind;
use tracing::warn;

use super::VideoFind;
use super::pick::pick;
use super::sources::{Failure, Lead, Wanted};
use super::ytdlp::Tube;
use crate::handlers::video_accounts::{Fetch, store};
use crate::objects::Blobs;

const SEARCH_RESULTS: usize = 5;
const PER_SEARCH: usize = 3;
const MIN_PAUSE: Duration = Duration::from_secs(60);

pub(super) struct Leased {
    pub(super) name: String,
    pub(super) cookies: String,
}

impl<F: Fetch + 'static, B: Blobs + 'static, T: Tube + 'static> VideoFind<F, B, T> {
    pub(super) async fn lease_youtube(&self) -> anyhow::Result<Option<Leased>> {
        Ok(store::lease(&self.pool, &self.seal, "youtube")
            .await?
            .map(|(name, cookies)| Leased { name, cookies }))
    }

    pub(super) async fn leads(&self, wanted: &Wanted, youtube: &Leased) -> Vec<Lead> {
        let mut leads = self.other_sources(wanted).await;
        leads.truncate(self.config.max_candidates);
        let room = self.config.max_candidates - leads.len();
        leads.extend(self.youtube_leads(wanted, youtube, room).await);
        leads
    }

    async fn youtube_leads(&self, wanted: &Wanted, youtube: &Leased, room: usize) -> Vec<Lead> {
        let base = format!("{} {}", wanted.artist, wanted.title);
        let duration_s = wanted.duration_ms.map(|ms| f64::from(ms) / 1000.0);
        let mut seen = HashSet::new();
        let mut leads = Vec::new();
        for query in [format!("{base} official video"), base] {
            if leads.len() >= room {
                break;
            }
            let found = match self
                .tube
                .search(&query, SEARCH_RESULTS, &youtube.cookies)
                .await
            {
                Ok(found) => found,
                Err(Failure::Paused(error, retry_after)) => {
                    let pause = pause_for(self.config.account_pause, retry_after);
                    self.account_out(&youtube.name, &error, Some(pause)).await;
                    break;
                }
                Err(Failure::Rejected(error)) => {
                    self.account_out(&youtube.name, &error, None).await;
                    break;
                }
                Err(Failure::Unavailable(error)) => {
                    warn!(account = %youtube.name, %error, "youtube search failed");
                    continue;
                }
            };
            for item in pick(found, &wanted.artist, &wanted.title, duration_s, PER_SEARCH) {
                if leads.len() < room && seen.insert(item.id.clone()) {
                    leads.push(Lead {
                        source: VideoSource::Youtube,
                        kind: VideoCandidateKind::Video,
                        url: item.url(),
                        source_ref: Some(item.id),
                    });
                }
            }
            tokio::time::sleep(self.config.youtube_pause).await;
        }
        leads
    }

    async fn other_sources(&self, wanted: &Wanted) -> Vec<Lead> {
        let mut found = Vec::new();
        if let Some((account, secret)) = self.lease("spotify").await {
            let answer = self.sources.canvas(&secret, wanted).await;
            found.extend(self.settled("spotify", &account, answer).await.flatten());
        }
        if let Some((account, secret)) = self.lease("yandex").await {
            let answer = self.sources.clips(&secret, wanted).await;
            found.extend(
                self.settled("yandex", &account, answer)
                    .await
                    .unwrap_or_default(),
            );
        }
        found
    }

    async fn lease(&self, source: &str) -> Option<(String, String)> {
        match store::lease(&self.pool, &self.seal, source).await {
            Ok(Some(leased)) => {
                tokio::time::sleep(self.config.source_gap).await;
                Some(leased)
            }
            Ok(None) => None,
            Err(error) => {
                warn!(source, %error, "video account lease failed");
                None
            }
        }
    }

    async fn settled<V>(
        &self,
        source: &str,
        account: &str,
        answer: Result<V, Failure>,
    ) -> Option<V> {
        match answer {
            Ok(value) => Some(value),
            Err(Failure::Paused(error, retry_after)) => {
                let pause = pause_for(self.config.account_pause, retry_after);
                self.account_out(account, &error, Some(pause)).await;
                None
            }
            Err(Failure::Rejected(error)) => {
                self.account_out(account, &error, None).await;
                None
            }
            Err(Failure::Unavailable(error)) => {
                warn!(source, account, %error, "video source is unavailable");
                None
            }
        }
    }

    pub(super) async fn account_out(&self, account: &str, error: &str, pause: Option<Duration>) {
        let outcome = match pause {
            Some(pause) => store::pause(&self.pool, account, pause, error).await,
            None => store::kill(&self.pool, account, error).await,
        };
        match outcome {
            Ok(()) => warn!(account, error, "video account taken out of rotation"),
            Err(failure) => warn!(account, %failure, "video account state was not saved"),
        }
    }
}

pub(super) fn pause_for(longest: Duration, retry_after: Option<u64>) -> Duration {
    retry_after.map_or(longest, |seconds| {
        Duration::from_secs(seconds).max(MIN_PAUSE).min(longest)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: Duration = Duration::from_secs(3_600);

    #[test]
    fn a_retry_after_sets_the_pause_within_a_minute_and_the_configured_cap() {
        assert_eq!(pause_for(HOUR, None), HOUR);
        assert_eq!(pause_for(HOUR, Some(900)), Duration::from_secs(900));
        assert_eq!(pause_for(HOUR, Some(5)), Duration::from_secs(60));
        assert_eq!(pause_for(HOUR, Some(86_400)), HOUR);
    }
}
