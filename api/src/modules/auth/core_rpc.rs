use std::sync::Arc;
use std::time::Duration;

use backend_contracts::auth::{
    AUTH_APP_TOKEN_SUBJECT, AUTH_REFRESH_SUBJECT, AppTokenReply, AppTokenRequest, RefreshReply,
    RefreshRequest,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tracing::warn;
use uuid::Uuid;

use crate::bus::nats::{NatsRequestError, NatsService};

const REFRESH_TIMEOUT: Duration = Duration::from_secs(5);
const APP_TOKEN_TIMEOUT: Duration = Duration::from_secs(2);

pub struct CoreRpc {
    nats: Option<Arc<NatsService>>,
}

impl CoreRpc {
    pub fn new(nats: Arc<NatsService>) -> Arc<Self> {
        Arc::new(Self { nats: Some(nats) })
    }

    #[cfg(test)]
    pub fn unreachable() -> Arc<Self> {
        Arc::new(Self { nats: None })
    }

    pub async fn refresh(&self, request: &RefreshRequest) -> Result<RefreshReply, NatsRequestError> {
        self.request(AUTH_REFRESH_SUBJECT, request, REFRESH_TIMEOUT)
            .await
    }

    pub async fn app_token(&self, oauth_app_id: Uuid) -> Result<AppTokenReply, NatsRequestError> {
        self.request(
            AUTH_APP_TOKEN_SUBJECT,
            &AppTokenRequest { oauth_app_id },
            APP_TOKEN_TIMEOUT,
        )
        .await
    }

    async fn request<P: Serialize, R: DeserializeOwned>(
        &self,
        subject: &str,
        payload: &P,
        timeout: Duration,
    ) -> Result<R, NatsRequestError> {
        let Some(nats) = &self.nats else {
            return Err(NatsRequestError::NoResponders(subject.to_owned()));
        };
        let reply = nats.request(subject, payload, timeout).await;
        if let Err(error) = &reply {
            warn!(%error, "core RPC failed");
        }
        reply
    }
}
