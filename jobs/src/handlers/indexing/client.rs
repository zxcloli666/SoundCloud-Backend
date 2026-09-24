use std::time::Duration;

use anyhow::{Context, anyhow};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::json;
use url::Url;
use wreq::{Client, Response, StatusCode};

use crate::config::IndexingConfig;
use crate::queue::{JobError, JobResult};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const RESPONSE_LIMIT: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TriggerOutcome {
    Accepted,
    Cached,
}

pub struct IndexingClient {
    client: Client,
    streaming_url: Url,
    internal_token: String,
}

#[derive(Deserialize)]
struct TranscodeResponse {
    cached: bool,
}

impl IndexingClient {
    pub fn new(config: &IndexingConfig) -> Result<Self, crate::ClientBuildError> {
        let client = sc_fingerprint::builder(None)
            .connect_timeout(Duration::from_secs(3))
            .timeout(REQUEST_TIMEOUT)
            .redirect(wreq::redirect::Policy::none())
            .pool_max_idle_per_host(16)
            .build()?;
        Ok(Self {
            client,
            streaming_url: config.streaming_url.clone(),
            internal_token: config.internal_token.expose().clone(),
        })
    }

    pub async fn trigger(&self, sc_track_id: &str) -> JobResult<TriggerOutcome> {
        let response = self
            .client
            .post(self.transcode_url(sc_track_id)?)
            .bearer_auth(&self.internal_token)
            .json(&json!({}))
            .send()
            .await
            .map_err(JobError::retryable)?;

        match response.status() {
            StatusCode::ACCEPTED => Ok(TriggerOutcome::Accepted),
            StatusCode::OK => {
                let response: TranscodeResponse = decode_response(response).await?;
                if response.cached {
                    Ok(TriggerOutcome::Cached)
                } else {
                    Err(JobError::retryable(anyhow!(
                        "streaming returned 200 without a cached track"
                    )))
                }
            }
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(JobError::retryable(anyhow!(
                "streaming rejected indexing authentication with {}",
                response.status()
            ))),
            status if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS => Err(
                JobError::retryable(anyhow!("streaming rejected indexing trigger with {status}")),
            ),
            status => Err(JobError::permanent(anyhow!(
                "streaming rejected indexing trigger with {status}"
            ))),
        }
    }

    fn transcode_url(&self, sc_track_id: &str) -> JobResult<Url> {
        let mut url = self.streaming_url.clone();
        let mut path = url.path_segments_mut().map_err(|_| {
            JobError::permanent(anyhow!("streaming URL cannot contain path segments"))
        })?;
        path.extend([
            "internal",
            "transcode-upload",
            &format!("soundcloud:tracks:{sc_track_id}"),
        ]);
        drop(path);
        Ok(url)
    }
}

async fn decode_response<T>(response: Response) -> JobResult<T>
where
    T: serde::de::DeserializeOwned,
{
    if response
        .content_length()
        .is_some_and(|length| length > RESPONSE_LIMIT as u64)
    {
        return Err(response_too_large());
    }

    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(JobError::retryable)?;
        if bytes.len().saturating_add(chunk.len()) > RESPONSE_LIMIT {
            return Err(response_too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .context("streaming response is not valid JSON")
        .map_err(JobError::retryable)
}

fn response_too_large() -> JobError {
    JobError::retryable(anyhow!("streaming response exceeds {RESPONSE_LIMIT} bytes"))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::post;

    use super::*;

    async fn start_server(
        response: impl axum::response::IntoResponse + Clone + Send + Sync + 'static,
    ) -> anyhow::Result<(Url, tokio::task::JoinHandle<()>)> {
        let app = Router::new().route(
            "/internal/transcode-upload/{track_urn}",
            post(move |headers: HeaderMap| {
                let response = response.clone();
                async move {
                    if headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        != Some("Bearer secret")
                    {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    response.into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok((format!("http://{address}").parse()?, task))
    }

    fn config(streaming_url: Url) -> IndexingConfig {
        IndexingConfig {
            streaming_url,
            internal_token: "secret".to_owned().into(),
        }
    }

    #[tokio::test]
    async fn trigger_sends_bearer_auth_and_reads_cached_response() -> anyhow::Result<()> {
        let (url, server) = start_server(axum::Json(json!({ "cached": true }))).await?;
        let client = IndexingClient::new(&config(url))?;

        let outcome = client.trigger("42").await?;

        server.abort();
        assert_eq!(outcome, TriggerOutcome::Cached);
        Ok(())
    }

    #[tokio::test]
    async fn oversized_response_is_retryable() -> anyhow::Result<()> {
        let (url, server) = start_server("x".repeat(RESPONSE_LIMIT + 1)).await?;
        let client = IndexingClient::new(&config(url))?;

        let error = match client.trigger("42").await {
            Err(error) => error,
            Ok(outcome) => anyhow::bail!("oversized response returned {outcome:?}"),
        };

        server.abort();
        assert!(error.is_retryable());
        Ok(())
    }

    #[test]
    fn service_paths_encode_the_track_urn() -> anyhow::Result<()> {
        let client = IndexingClient::new(&config("https://stream.example/base".parse()?))?;

        assert_eq!(
            client.transcode_url("42")?.as_str(),
            "https://stream.example/base/internal/transcode-upload/soundcloud:tracks:42"
        );
        Ok(())
    }
}
