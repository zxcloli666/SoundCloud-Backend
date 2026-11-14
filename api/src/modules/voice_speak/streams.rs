use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_nats::HeaderMap;
use async_nats::jetstream::stream::DirectGetErrorKind;
use backend_contracts::pipeline::{
    DEADLINE_HEADER, MSG_ID_HEADER, REPLY_TO_HEADER, SPEAK_REPLY_STREAM, SPEAK_RPC_STREAM,
    SPEAK_SAY,
};
use bytes::Bytes;
use futures::future::BoxFuture;

use super::stores::{Ask, Lane, Replies, StoreError};
use crate::bus::nats::NatsService;

const ACK_BUDGET: Duration = Duration::from_secs(3);
const READ_BUDGET: Duration = Duration::from_secs(2);

pub struct SpeakStreams {
    nats: Arc<NatsService>,
}

impl SpeakStreams {
    pub fn new(nats: Arc<NatsService>) -> Arc<Self> {
        Arc::new(Self { nats })
    }

    async fn queued(&self) -> Result<u64, StoreError> {
        let stream = self
            .nats
            .jetstream()
            .get_stream(SPEAK_RPC_STREAM.name)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        Ok(stream.cached_info().state.messages)
    }

    async fn publish(
        &self,
        subject: String,
        headers: HeaderMap,
        payload: Bytes,
    ) -> Result<(), StoreError> {
        let acknowledgement = self
            .nats
            .jetstream()
            .publish_with_headers(subject, headers, payload)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        acknowledgement
            .await
            .map(|_| ())
            .map_err(|_| StoreError::Unavailable)
    }

    async fn latest(&self, subject: &str) -> Result<Option<Bytes>, StoreError> {
        let stream = self
            .nats
            .jetstream()
            .get_stream_no_info(SPEAK_REPLY_STREAM.name)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        match stream.direct_get_last_for_subject(subject).await {
            Ok(message) => Ok(Some(message.payload)),
            Err(error) if error.kind() == DirectGetErrorKind::NotFound => Ok(None),
            Err(_) => Err(StoreError::Unavailable),
        }
    }
}

impl Lane for SpeakStreams {
    fn depth(&self) -> BoxFuture<'_, Result<u64, StoreError>> {
        Box::pin(within(READ_BUDGET, self.queued()))
    }

    fn ask(&self, ask: Ask) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            let payload = serde_json::to_vec(&ask.request).map_err(|_| StoreError::Unavailable)?;
            let sent = self.publish(SPEAK_SAY.to_owned(), headers(&ask), Bytes::from(payload));
            within(ACK_BUDGET, sent).await
        })
    }
}

impl Replies for SpeakStreams {
    fn last<'a>(&'a self, subject: &'a str) -> BoxFuture<'a, Result<Option<Bytes>, StoreError>> {
        Box::pin(within(READ_BUDGET, self.latest(subject)))
    }

    fn put<'a>(
        &'a self,
        subject: &'a str,
        payload: Bytes,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        let sent = self.publish(subject.to_owned(), HeaderMap::new(), payload);
        Box::pin(within(ACK_BUDGET, sent))
    }
}

pub(super) fn headers(ask: &Ask) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(REPLY_TO_HEADER, ask.reply_to.as_str());
    headers.insert(MSG_ID_HEADER, ask.ticket.to_string().as_str());
    headers.insert(DEADLINE_HEADER, epoch_millis(ask.deadline).as_str());
    headers
}

fn epoch_millis(at: SystemTime) -> String {
    at.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

async fn within<T>(
    budget: Duration,
    work: impl Future<Output = Result<T, StoreError>>,
) -> Result<T, StoreError> {
    tokio::time::timeout(budget, work)
        .await
        .map_err(|_| StoreError::TimedOut)?
}

#[cfg(test)]
#[path = "streams_tests.rs"]
mod tests;
