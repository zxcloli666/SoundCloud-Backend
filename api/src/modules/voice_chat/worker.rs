use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_nats::HeaderMap;
use backend_contracts::pipeline::{DEADLINE_HEADER, MSG_ID_HEADER, REPLY_TO_HEADER, RpcError};
use bytes::Bytes;
use futures::StreamExt;
use futures::future::BoxFuture;
use llm_ladder::{WorkerError, WorkerTransport};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::bus::nats::NatsService;

const PUBLISH_BUDGET: Duration = Duration::from_secs(5);
const MAX_REASON_LEN: usize = 48;

pub struct WorkerRpc {
    nats: Arc<NatsService>,
}

#[derive(Deserialize)]
struct RpcReply {
    ok: bool,
    data: Option<Value>,
    error: Option<String>,
}

impl WorkerRpc {
    pub fn new(nats: Arc<NatsService>) -> Arc<Self> {
        Arc::new(Self { nats })
    }

    async fn ask(
        &self,
        subject: &str,
        payload: Bytes,
        timeout: Duration,
    ) -> Result<Value, WorkerError> {
        let jetstream = self.nats.jetstream();
        let client = jetstream.client();
        let inbox = client.new_inbox();
        let mut replies = client
            .subscribe(inbox.clone())
            .await
            .map_err(|_| WorkerError::Unreachable)?;
        replies
            .unsubscribe_after(1)
            .await
            .map_err(|_| WorkerError::Unreachable)?;

        let mut headers = HeaderMap::new();
        headers.insert(REPLY_TO_HEADER, inbox.as_str());
        headers.insert(MSG_ID_HEADER, Uuid::now_v7().to_string().as_str());
        headers.insert(
            DEADLINE_HEADER,
            deadline(SystemTime::now(), timeout).as_str(),
        );
        let published = async {
            let acknowledgement = jetstream
                .publish_with_headers(subject.to_owned(), headers, payload)
                .await
                .map_err(|_| WorkerError::Unreachable)?;
            acknowledgement.await.map_err(|_| WorkerError::Unreachable)
        };
        tokio::time::timeout(PUBLISH_BUDGET.min(timeout), published)
            .await
            .map_err(|_| WorkerError::Unreachable)??;

        let reply = tokio::time::timeout(timeout, replies.next())
            .await
            .map_err(|_| WorkerError::Timeout)?
            .ok_or(WorkerError::Unreachable)?;
        let reply: RpcReply =
            serde_json::from_slice(&reply.payload).map_err(|_| WorkerError::Rejected)?;
        match (reply.ok, reply.data) {
            (true, Some(data)) => Ok(data),
            _ => {
                let error = reply.error.unwrap_or_default();
                tracing::warn!(
                    subject,
                    reason = reason(&error),
                    "the worker refused a completion"
                );
                Err(refusal(&error))
            }
        }
    }
}

impl WorkerTransport for WorkerRpc {
    fn call<'a>(
        &'a self,
        subject: &'a str,
        payload: Bytes,
        timeout: Duration,
    ) -> BoxFuture<'a, Result<Value, WorkerError>> {
        Box::pin(self.ask(subject, payload, timeout))
    }
}

fn deadline(now: SystemTime, timeout: Duration) -> String {
    let at = now.checked_add(timeout).unwrap_or(now);
    at.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string()
}

fn refusal(error: &str) -> WorkerError {
    match serde_json::from_value(Value::from(error)) {
        Ok(RpcError::InvalidRequest) => WorkerError::Invalid,
        Ok(RpcError::Expired) => WorkerError::Timeout,
        Ok(RpcError::Internal) | Err(_) => WorkerError::Rejected,
    }
}

fn reason(error: &str) -> String {
    error
        .chars()
        .map(|symbol| if symbol.is_whitespace() { '_' } else { symbol })
        .filter(|symbol| symbol.is_ascii_alphanumeric() || matches!(symbol, '_' | '-' | '.'))
        .take(MAX_REASON_LEN)
        .collect()
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
