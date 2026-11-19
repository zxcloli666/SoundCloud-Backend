use std::sync::Arc;
use std::time::Duration;

use async_nats::ConnectOptions;
use backend_contracts::auth::{
    AUTH_REFRESH_SUBJECT, RefreshOutcome, RefreshReply, RefreshRequest, RefreshTarget,
};
use bytes::Bytes;
use futures::future::BoxFuture;
use redact::Secret;
use tracing::{info, warn};
use url::Url;
use uuid::Uuid;

const REFRESH_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) type TokenRefresh =
    Arc<dyn Fn(Uuid, Option<String>) -> BoxFuture<'static, Option<Secret<String>>> + Send + Sync>;

pub(crate) async fn token_refresh(nats_url: &str) -> TokenRefresh {
    let Some(client) = connect(nats_url).await else {
        return Arc::new(|_, _| Box::pin(async { None }));
    };
    Arc::new(move |connection_id, rejected| {
        let client = client.clone();
        Box::pin(async move { ask_core(&client, connection_id, rejected).await })
    })
}

async fn connect(nats_url: &str) -> Option<async_nats::Client> {
    if nats_url.is_empty() {
        info!("[mobile] NATS_URL is empty, a stale mobile token is not refreshed");
        return None;
    }
    let parsed = Url::parse(nats_url)
        .inspect_err(|error| warn!("[mobile] NATS_URL is not a url: {error}"))
        .ok()?;
    let host = parsed.host_str()?;
    let address = match parsed.port() {
        Some(port) => format!("{}://{host}:{port}", parsed.scheme()),
        None => format!("{}://{host}", parsed.scheme()),
    };
    let mut options = ConnectOptions::new()
        .name("streaming")
        .max_reconnects(None)
        .retry_on_initial_connect();
    if !parsed.username().is_empty() {
        options = options.user_and_password(
            parsed.username().to_owned(),
            parsed.password().unwrap_or_default().to_owned(),
        );
    }
    match options.connect(address.as_str()).await {
        Ok(client) => {
            info!("[mobile] NATS client configured for {address}");
            Some(client)
        }
        Err(error) => {
            warn!("[mobile] NATS connect failed: {error}");
            None
        }
    }
}

async fn ask_core(
    client: &async_nats::Client,
    connection_id: Uuid,
    rejected: Option<String>,
) -> Option<Secret<String>> {
    let request = RefreshRequest {
        connection_id,
        rejected_token_fingerprint: rejected,
        hold: false,
        target: RefreshTarget::Mobile,
    };
    let body = serde_json::to_vec(&request).ok()?;
    let request = async_nats::Request::new()
        .payload(Bytes::from(body))
        .timeout(Some(REFRESH_TIMEOUT));
    let reply = client
        .send_request(AUTH_REFRESH_SUBJECT.to_owned(), request)
        .await
        .inspect_err(|error| warn!("[mobile] core refresh failed: {error}"))
        .ok()?;
    let reply: RefreshReply = serde_json::from_slice(&reply.payload)
        .inspect_err(|error| warn!("[mobile] core refresh reply unreadable: {error}"))
        .ok()?;
    usable(reply)
}

fn usable(reply: RefreshReply) -> Option<Secret<String>> {
    if !matches!(
        reply.outcome,
        RefreshOutcome::Refreshed | RefreshOutcome::AlreadyFresh
    ) {
        return None;
    }
    reply.mobile?.access_token.map(Secret::new)
}

#[cfg(test)]
mod tests {
    use backend_contracts::auth::MobileState;

    use super::*;

    fn reply(outcome: RefreshOutcome, token: Option<&str>) -> RefreshReply {
        RefreshReply {
            outcome,
            connection: None,
            retry_after_sec: None,
            mobile: Some(MobileState {
                status: "active".to_owned(),
                expires_at_unix_ms: 0,
                refresh_lease_expires_at_unix_ms: None,
                retry_at_unix_ms: None,
                last_refresh_error_kind: None,
                access_token: token.map(str::to_owned),
            }),
        }
    }

    #[test]
    fn only_a_refreshed_or_fresh_answer_with_a_token_is_used() {
        let refreshed = usable(reply(RefreshOutcome::Refreshed, Some("new")));
        let fresh = usable(reply(RefreshOutcome::AlreadyFresh, Some("kept")));

        assert_eq!(
            refreshed.as_ref().map(Secret::expose).map(String::as_str),
            Some("new")
        );
        assert_eq!(
            fresh.as_ref().map(Secret::expose).map(String::as_str),
            Some("kept")
        );
        for outcome in [
            RefreshOutcome::ReauthorizationRequired,
            RefreshOutcome::InProgress,
            RefreshOutcome::RetryLater,
            RefreshOutcome::RateLimited,
            RefreshOutcome::TimedOut,
            RefreshOutcome::NotConnected,
        ] {
            assert!(usable(reply(outcome, Some("t"))).is_none(), "{outcome:?}");
        }
        assert!(
            usable(reply(RefreshOutcome::Refreshed, None)).is_none(),
            "an older core answers without a token, so streaming moves on"
        );
    }
}
