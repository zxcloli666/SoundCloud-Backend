use std::time::Duration;

use backend_contracts::pipeline::RpcError;
use bytes::Bytes;
use futures::future::BoxFuture;
use llm_ladder::{WorkerError, WorkerTransport};
use serde_json::Value;
use uuid::Uuid;

use crate::bus::{Bus, RequestError};

pub struct BusWorker {
    bus: Bus,
}

impl BusWorker {
    pub fn new(bus: Bus) -> Self {
        Self { bus }
    }
}

impl WorkerTransport for BusWorker {
    fn call<'a>(
        &'a self,
        subject: &'a str,
        payload: Bytes,
        timeout: Duration,
    ) -> BoxFuture<'a, Result<Value, WorkerError>> {
        Box::pin(async move {
            let message_id = Uuid::now_v7().to_string();
            self.bus
                .request(subject, payload, timeout, &message_id)
                .await
                .map_err(worker_error)
        })
    }
}

fn worker_error(error: RequestError) -> WorkerError {
    match error {
        RequestError::Unreachable => WorkerError::Unreachable,
        RequestError::TimedOut => WorkerError::Timeout,
        RequestError::Malformed => WorkerError::Rejected,
        RequestError::Refused(reason) => match serde_json::from_value(Value::from(reason)) {
            Ok(RpcError::InvalidRequest) => WorkerError::Invalid,
            Ok(RpcError::Expired) => WorkerError::Timeout,
            Ok(RpcError::Internal) | Err(_) => WorkerError::Rejected,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worker_refusal_reads_like_the_api_reads_it() {
        let refused = |reason: &str| worker_error(RequestError::Refused(reason.to_owned()));

        assert_eq!(refused("invalid_request"), WorkerError::Invalid);
        assert_eq!(refused("expired"), WorkerError::Timeout);
        assert_eq!(refused("internal"), WorkerError::Rejected);
        assert_eq!(refused(""), WorkerError::Rejected);
        assert_eq!(worker_error(RequestError::TimedOut), WorkerError::Timeout);
        assert_eq!(
            worker_error(RequestError::Unreachable),
            WorkerError::Unreachable
        );
        assert_eq!(worker_error(RequestError::Malformed), WorkerError::Rejected);
    }
}
