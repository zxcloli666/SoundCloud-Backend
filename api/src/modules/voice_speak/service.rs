use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use backend_contracts::pipeline::{
    SAY_WINDOW_SECONDS, SPEAK_KEEP_PREFIX, SPEAK_REPLY_PREFIX, SayRequest,
};
use bytes::Bytes;
use uuid::Uuid;

use super::phrases::S3Phrases;
use super::reply::{Heard, heard};
use super::request::Line;
use super::stores::{Ask, Lane, Objects, Replies, StoreError};
use super::streams::SpeakStreams;
use crate::bus::nats::NatsService;
use crate::config::VoiceSpeakCfg;

const SAY_WINDOW: Duration = Duration::from_secs(SAY_WINDOW_SECONDS);
const LATE_GRACE: Duration = Duration::from_secs(5);
const KEEP_BUDGET: Duration = Duration::from_secs(2);

pub struct VoiceSpeak {
    live: Option<Live>,
}

pub struct Hosting {
    pub lane: Arc<dyn Lane>,
    pub replies: Arc<dyn Replies>,
    pub phrases: Option<Arc<dyn Objects>>,
}

#[derive(Clone, Copy, Debug)]
pub struct Hold {
    pub length: Duration,
    pub every: Duration,
}

pub struct Live {
    hosting: Hosting,
    max_in_flight: u64,
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
    Expired,
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
        let streams = SpeakStreams::new(nats.clone());
        let hosting = Hosting {
            lane: streams.clone(),
            replies: streams,
            phrases,
        };
        Self::hosted(config.max_in_flight, hosting, Hold::POLL)
    }

    pub fn hosted(max_in_flight: usize, hosting: Hosting, hold: Hold) -> Arc<Self> {
        let live = Live {
            hosting,
            max_in_flight: u64::try_from(max_in_flight).unwrap_or(u64::MAX),
            hold,
        };
        Arc::new(Self { live: Some(live) })
    }

    pub fn off() -> Arc<Self> {
        Arc::new(Self { live: None })
    }

    pub fn live(&self) -> Option<&Live> {
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
    pub async fn say(&self, user: &str, line: Line) -> Result<Said, Busy> {
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
        let asked = if self.has_room().await {
            self.ask(user, line, keep).await
        } else {
            Err(Busy)
        };
        crate::metrics::record_speak_line(if asked.is_ok() { "asked" } else { "busy" });
        asked.map(Said::Ticket)
    }

    pub async fn fetch(&self, user: &str, ticket: Uuid) -> Fetched {
        let subject = subject(SPEAK_REPLY_PREFIX, user, ticket);
        let fetched =
            match tokio::time::timeout(self.hold.length, self.wait(&subject, ticket)).await {
                Ok(Some(reply)) => self.answer(user, ticket, &reply).await,
                Ok(None) => Fetched::Expired,
                Err(_) => Fetched::Pending,
            };
        crate::metrics::record_speak_poll(fetched.label());
        fetched
    }

    async fn cached(&self, key: &str) -> Option<Bytes> {
        let phrases = self.hosting.phrases.as_ref()?;
        match phrases.read(key).await {
            Ok(Some(clip)) if !clip.is_empty() => Some(clip),
            Ok(_) => None,
            Err(error) => {
                failed("phrases", "read", error);
                None
            }
        }
    }

    async fn has_room(&self) -> bool {
        match self.hosting.lane.depth().await {
            Ok(depth) => depth < self.max_in_flight,
            Err(error) => {
                failed("lane", "depth", error);
                false
            }
        }
    }

    async fn ask(&self, user: &str, line: Line, keep: Option<String>) -> Result<Uuid, Busy> {
        let ticket = Uuid::now_v7();
        if let Some(key) = keep {
            let kept = subject(SPEAK_KEEP_PREFIX, user, ticket);
            self.hosting
                .replies
                .put(&kept, Bytes::from(key))
                .await
                .map_err(|error| refused("replies", "keep", error))?;
        }
        let ask = Ask {
            request: SayRequest {
                voice: line.voice.to_owned(),
                text: line.text,
            },
            reply_to: subject(SPEAK_REPLY_PREFIX, user, ticket),
            ticket,
            deadline: issued(ticket).unwrap_or_else(SystemTime::now) + SAY_WINDOW,
        };
        self.hosting
            .lane
            .ask(ask)
            .await
            .map_err(|error| refused("lane", "ask", error))?;
        Ok(ticket)
    }

    async fn wait(&self, subject: &str, ticket: Uuid) -> Option<Bytes> {
        let mut warned = false;
        loop {
            match self.hosting.replies.last(subject).await {
                Ok(Some(reply)) => return Some(reply),
                Ok(None) if expired(ticket, SystemTime::now()) => return None,
                Ok(None) => {}
                Err(error) => {
                    crate::metrics::record_speak_store_error("replies", "read", error.as_str());
                    if !warned {
                        tracing::warn!(error = error.as_str(), "a voiced line could not be read");
                        warned = true;
                    }
                }
            }
            tokio::time::sleep(self.hold.every).await;
        }
    }

    async fn answer(&self, user: &str, ticket: Uuid, reply: &[u8]) -> Fetched {
        let waited = issued(ticket)
            .and_then(|at| SystemTime::now().duration_since(at).ok())
            .unwrap_or_default();
        match heard(reply) {
            Heard::Clip { clip, seconds } => {
                crate::metrics::record_speak_reply("ok", waited, Some(seconds));
                self.keep(user, ticket, &clip).await;
                Fetched::Clip(clip)
            }
            Heard::Failed(reason) => {
                crate::metrics::record_speak_reply(reason, waited, None);
                tracing::warn!(reason, "the worker did not voice a line");
                Fetched::Failed
            }
        }
    }

    async fn keep(&self, user: &str, ticket: Uuid, clip: &Bytes) {
        let Some(phrases) = &self.hosting.phrases else {
            return;
        };
        let kept = subject(SPEAK_KEEP_PREFIX, user, ticket);
        let key = match self.hosting.replies.last(&kept).await {
            Ok(Some(key)) => key,
            Ok(None) => return,
            Err(error) => {
                failed("replies", "read", error);
                return;
            }
        };
        let Ok(key) = std::str::from_utf8(&key) else {
            crate::metrics::record_speak_store_error("replies", "read", "malformed");
            tracing::warn!("the key of a kept line is not text");
            return;
        };
        let written = tokio::time::timeout(KEEP_BUDGET, phrases.write(key, clip.clone()))
            .await
            .unwrap_or(Err(StoreError::TimedOut));
        if let Err(error) = written {
            failed("phrases", "write", error);
        }
    }
}

impl Fetched {
    fn label(&self) -> &'static str {
        match self {
            Self::Clip(_) => "clip",
            Self::Failed => "failed",
            Self::Expired => "expired",
            Self::Pending => "pending",
        }
    }
}

fn failed(store: &'static str, action: &'static str, error: StoreError) {
    crate::metrics::record_speak_store_error(store, action, error.as_str());
    tracing::warn!(
        store,
        action,
        error = error.as_str(),
        "a store of server voicing failed"
    );
}

fn refused(store: &'static str, action: &'static str, error: StoreError) -> Busy {
    failed(store, action, error);
    Busy
}

fn subject(prefix: &str, user: &str, ticket: Uuid) -> String {
    format!("{prefix}.{user}.{ticket}")
}

fn issued(ticket: Uuid) -> Option<SystemTime> {
    let (seconds, nanos) = ticket.get_timestamp()?.to_unix();
    Some(UNIX_EPOCH + Duration::new(seconds, nanos))
}

fn expired(ticket: Uuid, now: SystemTime) -> bool {
    issued(ticket)
        .is_none_or(|at| now.duration_since(at).unwrap_or_default() > SAY_WINDOW + LATE_GRACE)
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
