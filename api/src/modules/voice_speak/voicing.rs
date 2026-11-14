use std::sync::Arc;
use std::time::{Duration, Instant};

use backend_contracts::pipeline::{SAY_WINDOW_SECONDS, SPEAK_SAY, SayData, SayRequest};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use llm_ladder::WorkerError;
use tokio::sync::OwnedSemaphorePermit;

use super::service::Live;

const OGG_MAGIC: &[u8] = b"OggS";

pub struct Errand {
    pub clip: String,
    pub keep: Option<String>,
}

struct Voiced {
    clip: Bytes,
    seconds: f64,
}

impl Live {
    pub(super) async fn voice(
        self: Arc<Self>,
        _permit: OwnedSemaphorePermit,
        request: SayRequest,
        errand: Errand,
    ) {
        let started = Instant::now();
        match self.ask(&request).await {
            Ok(voiced) => {
                crate::metrics::record_speak_reply("ok", started.elapsed(), Some(voiced.seconds));
                self.hand_over(errand, voiced.clip).await;
            }
            Err(failure) => {
                crate::metrics::record_speak_reply(failure, started.elapsed(), None);
                tracing::warn!(
                    reason = failure,
                    text_chars = request.text.chars().count(),
                    "the worker did not voice a line"
                );
                self.mark_failed(&errand.clip).await;
            }
        }
    }

    async fn ask(&self, request: &SayRequest) -> Result<Voiced, &'static str> {
        let payload = serde_json::to_vec(request).map_err(|_| "unencodable")?;
        let data = self
            .hosting
            .worker
            .call(
                SPEAK_SAY,
                Bytes::from(payload),
                Duration::from_secs(SAY_WINDOW_SECONDS),
            )
            .await
            .map_err(failure)?;
        let data: SayData = serde_json::from_value(data).map_err(|_| "malformed")?;
        let clip = ogg(&data.clip).ok_or("malformed")?;
        Ok(Voiced {
            clip,
            seconds: data.seconds,
        })
    }

    async fn hand_over(&self, errand: Errand, clip: Bytes) {
        let bytes = clip.len();
        if let Err(error) = self.hosting.clips.write(&errand.clip, clip.clone()).await {
            crate::metrics::record_speak_store_error("clips", "write", error.as_str());
            tracing::warn!(
                error = error.as_str(),
                bytes,
                "a voiced line could not be handed to the poll"
            );
            self.mark_failed(&errand.clip).await;
        }
        if let Some(key) = errand.keep
            && let Err(error) = self.hosting.phrases.write(&key, clip).await
        {
            crate::metrics::record_speak_store_error("phrases", "write", error.as_str());
            tracing::warn!(
                error = error.as_str(),
                bytes,
                "a voiced line could not be kept"
            );
        }
    }

    async fn mark_failed(&self, clip: &str) {
        if let Err(error) = self.hosting.clips.write(clip, Bytes::new()).await {
            crate::metrics::record_speak_store_error("clips", "mark", error.as_str());
            tracing::warn!(
                error = error.as_str(),
                "the failure of a line could not be marked"
            );
        }
    }
}

fn ogg(encoded: &str) -> Option<Bytes> {
    let clip = STANDARD.decode(encoded).ok()?;
    clip.starts_with(OGG_MAGIC).then(|| Bytes::from(clip))
}

fn failure(error: WorkerError) -> &'static str {
    match error {
        WorkerError::Timeout => "timeout",
        WorkerError::Unreachable => "unreachable",
        WorkerError::Rejected => "rejected",
        WorkerError::Invalid => "invalid",
    }
}
