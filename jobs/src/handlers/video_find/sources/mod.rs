mod spotify;
mod token;
mod yandex;

use backend_contracts::admin_video::VideoSource;
use backend_contracts::pipeline::VideoCandidateKind;
use bytes::Bytes;
use catalog_sources::SourceError;

use self::token::Tokens;
use crate::handlers::video_accounts::{Fetch, SpotifyKey, Verdict};

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Lead {
    pub(super) source: VideoSource,
    pub(super) kind: VideoCandidateKind,
    pub(super) url: String,
    pub(super) source_ref: Option<String>,
}

pub(super) struct Wanted {
    pub(super) artist: String,
    pub(super) title: String,
    pub(super) isrc: Option<String>,
    pub(super) duration_ms: Option<i32>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    Paused(String, Option<u64>),
    Rejected(String),
    Unavailable(String),
}

pub(super) struct Sources<F> {
    fetch: F,
    spotify: SpotifyKey,
    tokens: Tokens,
}

impl<F: Fetch> Sources<F> {
    pub(super) fn new(fetch: F, spotify: SpotifyKey) -> Self {
        Self {
            fetch,
            spotify,
            tokens: Tokens::default(),
        }
    }

    pub(super) async fn canvas(
        &self,
        sp_dc: &str,
        wanted: &Wanted,
    ) -> Result<Option<Lead>, Failure> {
        spotify::canvas(&self.fetch, &self.tokens, &self.spotify, sp_dc, wanted).await
    }

    pub(super) async fn clips(&self, token: &str, wanted: &Wanted) -> Result<Vec<Lead>, Failure> {
        yandex::clips(&self.fetch, token, wanted).await
    }

    pub(super) async fn file(&self, url: &str) -> Result<Bytes, Failure> {
        self.fetch
            .get(url, wreq::header::HeaderMap::new())
            .await
            .map_err(|error| failure_of("canvas", &error))
    }
}

impl From<Verdict> for Failure {
    fn from(verdict: Verdict) -> Self {
        match verdict {
            Verdict::Rejected(error) => Self::Rejected(error),
            Verdict::Limited(error, retry_after) => Self::Paused(error, retry_after),
            Verdict::Unavailable(error) => Self::Unavailable(error),
            Verdict::Valid => Self::Unavailable("the account answered as valid".to_owned()),
        }
    }
}

fn failure_of(source: &str, error: &SourceError) -> Failure {
    match error.status_code() {
        Some(429) => Failure::Paused(
            format!("{source} answered 429"),
            error.retry_after_seconds(),
        ),
        Some(status @ (401 | 403)) => Failure::Rejected(format!("{source} answered {status}")),
        _ => Failure::Unavailable(format!("{source}: {error}")),
    }
}
