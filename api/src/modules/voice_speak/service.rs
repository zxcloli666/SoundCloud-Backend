use std::sync::Arc;
use std::time::Duration;

use backend_contracts::pipeline::{SPEAK_CLIPS_BUCKET, SayRequest};
use bytes::Bytes;
use llm_ladder::WorkerTransport;
use tokio::sync::Semaphore;
use uuid::Uuid;

use super::objects::{NatsObjects, Objects};
use super::phrases::S3Phrases;
use super::request::Line;
use super::voicing::Errand;
use crate::bus::nats::NatsService;
use crate::bus::worker_rpc::WorkerRpc;
use crate::config::VoiceSpeakCfg;

pub struct VoiceSpeak {
    live: Option<Arc<Live>>,
}

pub struct Hosting {
    pub worker: Arc<dyn WorkerTransport>,
    pub clips: Arc<dyn Objects>,
    pub phrases: Option<Arc<dyn Objects>>,
}

#[derive(Clone, Copy, Debug)]
pub struct Hold {
    pub length: Duration,
    pub every: Duration,
}

pub struct Live {
    pub(super) hosting: Hosting,
    pub(super) lines: Arc<Semaphore>,
    hold: Hold,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Said {
    Cached(Bytes),
    Ticket(Uuid),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Fetched {
    Clip(Bytes),
    Failed,
    Pending,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Busy;

impl VoiceSpeak {
    pub fn start(config: Option<&VoiceSpeakCfg>, nats: &Arc<NatsService>) -> Arc<Self> {
        let Some(config) = config else {
            return Self::off();
        };
        let phrases = match config.phrases.as_ref().map(S3Phrases::connect).transpose() {
            Ok(phrases) => phrases.map(|phrases| phrases as Arc<dyn Objects>),
            Err(reason) => {
                tracing::error!(reason, "server voicing is off: the phrase cache is refused");
                return Self::off();
            }
        };
        let hosting = Hosting {
            worker: WorkerRpc::new(nats.clone()),
            clips: NatsObjects::new(nats.clone(), SPEAK_CLIPS_BUCKET),
            phrases,
        };
        Self::hosted(config.max_in_flight, hosting, Hold::POLL)
    }

    pub fn hosted(max_in_flight: usize, hosting: Hosting, hold: Hold) -> Arc<Self> {
        let live = Arc::new(Live {
            hosting,
            lines: Arc::new(Semaphore::new(max_in_flight)),
            hold,
        });
        Arc::new(Self { live: Some(live) })
    }

    pub fn off() -> Arc<Self> {
        Arc::new(Self { live: None })
    }

    pub fn live(&self) -> Option<&Arc<Live>> {
        self.live.as_ref()
    }
}

impl Hold {
    pub const POLL: Self = Self {
        length: Duration::from_secs(20),
        every: Duration::from_millis(500),
    };
}

impl Live {
    pub async fn say(self: &Arc<Self>, user: &str, line: Line) -> Result<Said, Busy> {
        let keep = match &self.hosting.phrases {
            Some(_) if line.keep => Some(line.phrase_key()),
            _ => None,
        };
        if let Some(key) = &keep
            && let Some(clip) = self.cached(key).await
        {
            crate::metrics::record_speak_line("cached");
            return Ok(Said::Cached(clip));
        }
        let Ok(permit) = self.lines.clone().try_acquire_owned() else {
            crate::metrics::record_speak_line("busy");
            return Err(Busy);
        };
        let ticket = Uuid::now_v7();
        let request = SayRequest {
            voice: line.voice.to_owned(),
            text: line.text,
        };
        let errand = Errand {
            clip: clip_name(user, ticket),
            keep,
        };
        tokio::spawn(Arc::clone(self).voice(permit, request, errand));
        crate::metrics::record_speak_line("asked");
        Ok(Said::Ticket(ticket))
    }

    pub async fn fetch(&self, user: &str, ticket: Uuid) -> Fetched {
        let name = clip_name(user, ticket);
        let fetched = tokio::time::timeout(self.hold.length, self.look(&name))
            .await
            .unwrap_or(Fetched::Pending);
        crate::metrics::record_speak_poll(fetched.label());
        fetched
    }

    async fn cached(&self, key: &str) -> Option<Bytes> {
        let phrases = self.hosting.phrases.as_ref()?;
        match phrases.read(key).await {
            Ok(Some(clip)) if !clip.is_empty() => Some(clip),
            Ok(_) => None,
            Err(error) => {
                crate::metrics::record_speak_store_error("phrases", "read", error.as_str());
                tracing::warn!(error = error.as_str(), "the phrase cache could not be read");
                None
            }
        }
    }

    async fn look(&self, name: &str) -> Fetched {
        let mut warned = false;
        loop {
            match self.hosting.clips.read(name).await {
                Ok(Some(clip)) if clip.is_empty() => return Fetched::Failed,
                Ok(Some(clip)) => return Fetched::Clip(clip),
                Ok(None) => {}
                Err(error) => {
                    crate::metrics::record_speak_store_error("clips", "read", error.as_str());
                    if !warned {
                        tracing::warn!(error = error.as_str(), "a voiced line could not be read");
                        warned = true;
                    }
                }
            }
            tokio::time::sleep(self.hold.every).await;
        }
    }
}

impl Fetched {
    fn label(&self) -> &'static str {
        match self {
            Self::Clip(_) => "clip",
            Self::Failed => "failed",
            Self::Pending => "pending",
        }
    }
}

fn clip_name(user: &str, ticket: Uuid) -> String {
    format!("{user}.{ticket}")
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
