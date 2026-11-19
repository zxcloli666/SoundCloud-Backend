use std::future::Future;

use backend_contracts::pipeline::VideoLabel;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::llm::anthropic::{Answer, Anthropic, Call, LlmError, Spend, finished, text_of};

pub const PROMPT: &str = "You see frames from a music video, sampled evenly in time and shown in order.
Classify the visual track into one class:
static = the same single picture the whole time. It may have a slow zoom or pan, a waveform or spectrum drawn over the still picture, or subtle overlays on the same picture (noise, grain, flicker, particles, color shifts).
loop = a short segment of real motion that repeats again and again; the same frames come back in a cycle.
clip = real footage or animation that changes over time, with different shots, cuts or scenes, and no short repetition.
Answer with one word: clip, loop or static.";

const CALL: &str = "vlm";
const EFFORT: &str = "low";

#[derive(Clone, Debug, PartialEq)]
pub struct Judged {
    pub label: VideoLabel,
    pub model: String,
}

pub trait Vision: Send + Sync {
    fn judge(&self, frames: &[Vec<u8>]) -> impl Future<Output = Answer<Judged>> + Send;
}

impl Vision for Anthropic {
    async fn judge(&self, frames: &[Vec<u8>]) -> Answer<Judged> {
        let messages = [request(frames)];
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
                .and_then(|()| label_of(&text_of(&message)))
                .map(|label| Judged {
                    label,
                    model: model_of(&spend),
                }),
            Err(error) => Err(error),
        };
        Answer { result, spend }
    }
}

pub fn request(frames: &[Vec<u8>]) -> Value {
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
    content.push(json!({"type": "text", "text": PROMPT}));
    json!({"role": "user", "content": content})
}

pub fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "label": {"type": "string", "enum": ["clip", "loop", "static"]},
        },
        "required": ["label"],
        "additionalProperties": false,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    label: VideoLabel,
}

pub fn label_of(text: &str) -> Result<VideoLabel, LlmError> {
    serde_json::from_str::<Reply>(text.trim())
        .map(|reply| reply.label)
        .map_err(|error| LlmError::Invalid(format!("label: {error}")))
}

fn model_of(spend: &[Spend]) -> String {
    spend
        .last()
        .map(|call| call.model.clone())
        .unwrap_or_default()
}
