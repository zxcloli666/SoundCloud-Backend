use std::sync::{Arc, Mutex};

use bytes::Bytes;
use catalog_sources::{SourceError, SourceResult};
use wreq::header::HeaderMap;

use super::Fetch;

#[derive(Clone)]
pub(crate) enum Answer {
    Body(String),
    Status(u16),
    Down,
}

#[derive(Default)]
pub(crate) struct Mock {
    routes: Mutex<Vec<(String, Answer)>>,
    seen: Mutex<Vec<(String, HeaderMap)>>,
}

impl Mock {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn route(&self, prefix: &str, answer: Answer) {
        if let Ok(mut routes) = self.routes.lock() {
            routes.push((prefix.to_owned(), answer));
        }
    }

    pub(crate) fn requests(&self) -> Vec<(String, HeaderMap)> {
        self.seen
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default()
    }
}

impl Fetch for Arc<Mock> {
    async fn get(&self, url: &str, headers: HeaderMap) -> SourceResult<Bytes> {
        self.answer(url, headers)
    }

    async fn post(&self, url: &str, headers: HeaderMap, _body: Bytes) -> SourceResult<Bytes> {
        self.answer(url, headers)
    }
}

impl Mock {
    fn answer(&self, url: &str, headers: HeaderMap) -> SourceResult<Bytes> {
        if let Ok(mut seen) = self.seen.lock() {
            seen.push((url.to_owned(), headers));
        }
        let answer = self.routes.lock().ok().and_then(|routes| {
            routes
                .iter()
                .rev()
                .find(|(prefix, _)| url.starts_with(prefix.as_str()))
                .map(|(_, answer)| answer.clone())
        });
        match answer {
            Some(Answer::Body(body)) => Ok(Bytes::from(body)),
            Some(Answer::Status(status)) => Err(SourceError::Status {
                status,
                body: String::new(),
                retry_after_seconds: None,
            }),
            Some(Answer::Down) | None => Err(SourceError::Unreachable("down".to_owned())),
        }
    }
}
