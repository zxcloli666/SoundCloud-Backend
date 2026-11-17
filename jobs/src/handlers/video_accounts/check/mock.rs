use std::sync::{Arc, Mutex};

use bytes::Bytes;
use catalog_sources::{SourceError, SourceResult};
use wreq::header::HeaderMap;

use super::Fetch;

#[derive(Clone)]
pub(in crate::handlers::video_accounts) enum Answer {
    Body(String),
    Status(u16),
    Down,
}

#[derive(Default)]
pub(in crate::handlers::video_accounts) struct Mock {
    routes: Mutex<Vec<(String, Answer)>>,
    seen: Mutex<Vec<(String, HeaderMap)>>,
}

impl Mock {
    pub(in crate::handlers::video_accounts) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(in crate::handlers::video_accounts) fn route(&self, prefix: &str, answer: Answer) {
        if let Ok(mut routes) = self.routes.lock() {
            routes.push((prefix.to_owned(), answer));
        }
    }

    pub(in crate::handlers::video_accounts) fn requests(&self) -> Vec<(String, HeaderMap)> {
        self.seen
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default()
    }
}

impl Fetch for Arc<Mock> {
    async fn get(&self, url: &str, headers: HeaderMap) -> SourceResult<Bytes> {
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
