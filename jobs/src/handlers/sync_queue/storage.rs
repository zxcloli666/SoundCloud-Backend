use std::time::Duration;

use wreq::{Client, StatusCode, Url};

use crate::config::SyncQueueConfig;

use super::model::ClaimedMutation;

const RETRY_DELAYS: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_millis(100),
    Duration::from_millis(250),
];

pub struct TrackStorage {
    client: Client,
    base_url: Url,
    token: String,
}

impl TrackStorage {
    pub fn new(config: &SyncQueueConfig) -> Result<Self, crate::ClientBuildError> {
        Ok(Self {
            client: sc_fingerprint::builder(None)
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(5))
                .build()?,
            base_url: config.storage_url.clone(),
            token: config.storage_token.expose().clone(),
        })
    }

    pub async fn evict_private(&self, mutation: &ClaimedMutation) -> anyhow::Result<()> {
        if !is_private_track(mutation) {
            return Ok(());
        }
        let url = self.delete_url(&mutation.target_urn)?;
        for (attempt, delay) in RETRY_DELAYS.into_iter().enumerate() {
            tokio::time::sleep(delay).await;
            match self
                .client
                .delete(url.clone())
                .bearer_auth(&self.token)
                .send()
                .await
            {
                Ok(response)
                    if response.status().is_success()
                        || matches!(
                            response.status(),
                            StatusCode::NOT_FOUND | StatusCode::GONE
                        ) =>
                {
                    return Ok(());
                }
                Ok(_) | Err(_) if attempt + 1 < RETRY_DELAYS.len() => {}
                Ok(response) => {
                    anyhow::bail!(
                        "track storage rejected eviction with status {}",
                        response.status().as_u16()
                    );
                }
                Err(error) => {
                    anyhow::bail!("track storage eviction failed: {}", error.without_url())
                }
            }
        }
        unreachable!()
    }

    fn delete_url(&self, track_urn: &str) -> anyhow::Result<Url> {
        let filename = track_urn.replace(':', "_");
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("track storage URL cannot be a base URL"))?
            .pop_if_empty()
            .extend(["files", &filename]);
        url.set_query(None);
        url.set_fragment(None);
        Ok(url)
    }
}

fn is_private_track(mutation: &ClaimedMutation) -> bool {
    mutation.action_type == "track_delete"
        || (mutation.action_type == "track_update"
            && mutation
                .payload
                .as_ref()
                .and_then(|payload| payload.pointer("/track/sharing"))
                .and_then(serde_json::Value::as_str)
                == Some("private"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_track_mutations_are_selected() {
        let mut mutation = mutation(Some(
            serde_json::json!({ "track": { "sharing": "private" } }),
        ));
        assert!(is_private_track(&mutation));

        mutation.payload = Some(serde_json::json!({ "track": { "sharing": "public" } }));
        assert!(!is_private_track(&mutation));

        mutation.action_type = "track_delete".into();
        mutation.payload = None;
        assert!(is_private_track(&mutation));
    }

    fn mutation(payload: Option<serde_json::Value>) -> ClaimedMutation {
        ClaimedMutation {
            id: uuid::Uuid::nil(),
            user_id: "user".to_owned(),
            action_type: "track_update".to_owned(),
            target_urn: "soundcloud:tracks:42".to_owned(),
            payload,
            retry_count: 0,
            generation: 1,
            lease_id: uuid::Uuid::nil(),
            lease_generation: 1,
            remote_attempted_generation: None,
            remote_completed_generation: None,
            remote_result: None,
        }
    }

    #[test]
    fn delete_url_encodes_a_canonical_track_urn() -> anyhow::Result<()> {
        let storage = TrackStorage {
            client: Client::new(),
            base_url: "https://storage.example/base/".parse()?,
            token: "token".into(),
        };
        assert_eq!(
            storage.delete_url("soundcloud:tracks:42")?.as_str(),
            "https://storage.example/base/files/soundcloud_tracks_42"
        );
        Ok(())
    }

    #[tokio::test]
    async fn deletion_evicts_storage_with_authenticated_idempotent_retries() -> anyhow::Result<()> {
        use axum::{
            Router,
            extract::{Path, State},
            http::HeaderMap,
            routing::delete,
        };
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route(
                "/files/{filename}",
                delete(
                    |State(calls): State<Arc<AtomicUsize>>,
                     Path(filename): Path<String>,
                     headers: HeaderMap| async move {
                        assert_eq!(filename, "soundcloud_tracks_42");
                        assert_eq!(
                            headers
                                .get("authorization")
                                .and_then(|value| value.to_str().ok()),
                            Some("Bearer secret")
                        );
                        if calls.fetch_add(1, Ordering::Relaxed) < 2 {
                            axum::http::StatusCode::SERVICE_UNAVAILABLE
                        } else {
                            axum::http::StatusCode::NOT_FOUND
                        }
                    },
                ),
            )
            .with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
        });
        let storage = TrackStorage {
            client: Client::new(),
            base_url: format!("http://{address}").parse()?,
            token: "secret".into(),
        };
        let mut deletion = mutation(None);
        deletion.action_type = "track_delete".into();
        let result = storage.evict_private(&deletion).await;
        let _ = stop.send(());
        server.await??;
        result?;
        assert_eq!(calls.load(Ordering::Relaxed), 3);
        Ok(())
    }
}
