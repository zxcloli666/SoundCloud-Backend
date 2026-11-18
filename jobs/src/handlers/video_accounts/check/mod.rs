#[cfg(test)]
pub(crate) mod mock;
mod spotify;
#[cfg(test)]
mod tests;
mod totp;
mod yandex;
mod youtube;

use std::future::Future;
use std::sync::Arc;

use backend_contracts::admin_video::VideoSource;
use bytes::Bytes;
use catalog_sources::{ExternalFetcher, SourceError, SourceResult};
use wreq::header::{HeaderMap, HeaderName, HeaderValue};

pub use spotify::SpotifyKey;
pub(crate) use spotify::access_token;

const PAGE_LIMIT: usize = 4 * 1024 * 1024;
pub(crate) const BROWSER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Valid,
    Rejected(String),
    Unavailable(String),
}

pub trait Fetch: Send + Sync {
    fn get(
        &self,
        url: &str,
        headers: HeaderMap,
    ) -> impl Future<Output = SourceResult<Bytes>> + Send;

    fn post(
        &self,
        url: &str,
        headers: HeaderMap,
        body: Bytes,
    ) -> impl Future<Output = SourceResult<Bytes>> + Send;
}

pub struct Proxied {
    fetcher: Arc<ExternalFetcher>,
}

impl Proxied {
    pub fn new(fetcher: Arc<ExternalFetcher>) -> Self {
        Self { fetcher }
    }
}

impl Fetch for Proxied {
    async fn get(&self, url: &str, headers: HeaderMap) -> SourceResult<Bytes> {
        self.fetcher
            .get_authenticated(url, headers, PAGE_LIMIT)
            .await
    }

    async fn post(&self, url: &str, headers: HeaderMap, body: Bytes) -> SourceResult<Bytes> {
        self.fetcher
            .post_authenticated(url, headers, body, PAGE_LIMIT)
            .await
    }
}

pub struct DirectFirst {
    fetcher: Arc<ExternalFetcher>,
}

impl DirectFirst {
    pub fn new(fetcher: Arc<ExternalFetcher>) -> Self {
        Self { fetcher }
    }
}

impl Fetch for DirectFirst {
    async fn get(&self, url: &str, headers: HeaderMap) -> SourceResult<Bytes> {
        self.fetcher
            .get_direct_first(url, headers, PAGE_LIMIT)
            .await
    }

    async fn post(&self, url: &str, headers: HeaderMap, body: Bytes) -> SourceResult<Bytes> {
        self.fetcher
            .post_direct_first(url, headers, body, PAGE_LIMIT)
            .await
    }
}

pub struct Checker<F> {
    fetch: F,
    spotify: SpotifyKey,
}

impl<F: Fetch> Checker<F> {
    pub fn new(fetch: F, spotify: SpotifyKey) -> Self {
        Self { fetch, spotify }
    }

    pub async fn check(&self, source: VideoSource, secret: &str) -> Verdict {
        let secret = secret.trim();
        match source {
            VideoSource::Youtube => youtube::check(&self.fetch, secret).await,
            VideoSource::Spotify => spotify::check(&self.fetch, &self.spotify, secret).await,
            VideoSource::Yandex => yandex::check(&self.fetch, secret).await,
        }
    }
}

pub(crate) fn headers(pairs: &[(&'static str, &str)]) -> Result<HeaderMap, Verdict> {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        let value = HeaderValue::from_str(value).map_err(|_| {
            Verdict::Rejected("the secret has characters a header cannot carry".to_owned())
        })?;
        map.insert(HeaderName::from_static(name), value);
    }
    Ok(map)
}

fn unavailable(source: &str, error: &SourceError) -> Verdict {
    match error {
        SourceError::Status { status, .. } => {
            Verdict::Unavailable(format!("{source} answered {status}"))
        }
        other => Verdict::Unavailable(format!("{source} is unreachable: {other}")),
    }
}
