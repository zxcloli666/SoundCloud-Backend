use std::time::{Duration, SystemTime};

use async_nats::HeaderMap;
use backend_contracts::pipeline::{DEADLINE_HEADER, MSG_ID_HEADER, REPLY_TO_HEADER};
use bytes::Bytes;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::Value;

use super::{Bus, PUBLISH_TIMEOUT, deadline_header};

#[derive(Debug, PartialEq, Eq)]
pub enum RequestError {
    Unreachable,
    TimedOut,
    Malformed,
    Refused(String),
}

#[derive(Deserialize)]
struct RpcReply {
    ok: bool,
    data: Option<Value>,
    error: Option<String>,
}

impl Bus {
    pub async fn request(
        &self,
        subject: &str,
        payload: Bytes,
        timeout: Duration,
        msg_id: &str,
    ) -> Result<Value, RequestError> {
        let inbox = self.client.new_inbox();
        let mut replies = self
            .client
            .subscribe(inbox.clone())
            .await
            .map_err(|_| RequestError::Unreachable)?;
        replies
            .unsubscribe_after(1)
            .await
            .map_err(|_| RequestError::Unreachable)?;

        let mut headers = HeaderMap::new();
        headers.insert(REPLY_TO_HEADER, inbox.as_str());
        headers.insert(MSG_ID_HEADER, msg_id);
        headers.insert(
            DEADLINE_HEADER,
            deadline_header(SystemTime::now(), timeout).as_str(),
        );
        let published = async {
            let acknowledgement = self
                .jetstream
                .publish_with_headers(subject.to_owned(), headers, payload)
                .await
                .map_err(|_| RequestError::Unreachable)?;
            acknowledgement.await.map_err(|_| RequestError::Unreachable)
        };
        tokio::time::timeout(PUBLISH_TIMEOUT.min(timeout), published)
            .await
            .map_err(|_| RequestError::Unreachable)??;

        let reply = tokio::time::timeout(timeout, replies.next())
            .await
            .map_err(|_| RequestError::TimedOut)?
            .ok_or(RequestError::Unreachable)?;
        let reply: RpcReply =
            serde_json::from_slice(&reply.payload).map_err(|_| RequestError::Malformed)?;
        match (reply.ok, reply.data) {
            (true, Some(data)) => Ok(data),
            _ => Err(RequestError::Refused(reply.error.unwrap_or_default())),
        }
    }
}
