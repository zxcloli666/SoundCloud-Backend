use std::future::Future;

use backend_contracts::pipeline::VideoLabel;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};

use super::prompt::{self, Features};
use crate::llm::anthropic::{Answer, Anthropic, Call, LlmError, Spend, finished, text_of};

const CALL: &str = "vlm";
const EFFORT: &str = "low";

#[derive(Clone, Debug, PartialEq)]
pub struct Judged {
    pub label: VideoLabel,
    pub model: String,
    pub reason: String,
}

pub trait Vision: Send + Sync {
    fn judge(
        &self,
        frames: &[Vec<u8>],
        features: &Features,
    ) -> impl Future<Output = Answer<Judged>> + Send;
}

impl Vision for Anthropic {
    async fn judge(&self, frames: &[Vec<u8>], features: &Features) -> Answer<Judged> {
        let messages = [request(frames, &prompt::remote(frames.len(), features))];
        let call = Call {
            name: CALL,
            system: None,
            messages: &messages,
            tools: None,
            format: Some(schema()),
            effort: Some(EFFORT),
        };
        let mut spend = Vec::new();
        let result = match self.send(&call, &mut spend).await {
            Ok(message) => finished(&message)
                .and_then(|()| reply_of(&text_of(&message)))
                .map(|reply| Judged {
                    label: reply.label,
                    model: model_of(&spend),
                    reason: reply.reason,
                }),
            Err(error) => Err(error),
        };
        Answer { result, spend }
    }
}

pub fn request(frames: &[Vec<u8>], text: &str) -> Value {
    let mut content: Vec<Value> = frames
        .iter()
        .map(|frame| {
            json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": "image/jpeg",
                    "data": base64::engine::general_purpose::STANDARD.encode(frame),
                },
            })
        })
        .collect();
    content.push(json!({"type": "text", "text": text}));
    json!({"role": "user", "content": content})
}

pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "reason": {"type": "string"},
            "label": {"type": "string", "enum": ["clip", "loop", "static"]},
        },
        "required": ["reason", "label"],
        "additionalProperties": false,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    #[serde(default)]
    pub reason: String,
    pub label: VideoLabel,
}

pub fn reply_of(text: &str) -> Result<Reply, LlmError> {
    serde_json::from_str::<Reply>(text.trim())
        .map_err(|error| LlmError::Invalid(format!("label: {error}")))
}

fn model_of(spend: &[Spend]) -> String {
    spend
        .last()
        .map(|call| call.model.clone())
        .unwrap_or_default()
}
