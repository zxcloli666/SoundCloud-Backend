use base64::Engine as _;
use serde_json::{Value, json};

use crate::handlers::video_gen::director::{Brief, Picture, Plan, Review, Verdict};
use crate::handlers::video_gen::prompts;
use crate::llm::anthropic::{Anthropic, Call, LlmError, Spend, finished, text_of};

const MAX_TURNS: usize = 6;
const NUDGE: &str = "Answer by calling accept or reject.";

pub(super) async fn plan(
    opus: &Anthropic,
    brief: &Brief,
    spend: &mut Vec<Spend>,
) -> Result<Plan, LlmError> {
    let messages = [json!({
        "role": "user",
        "content": [
            {"type": "image", "source": {"type": "url", "url": brief.cover_url}},
            {"type": "text", "text": prompts::plan_request(brief)},
        ],
    })];
    let call = Call {
        name: "plan",
        system: Some(prompts::SYSTEM),
        messages: &messages,
        tools: None,
        format: Some(prompts::plan_schema()),
        effort: None,
    };
    let message = opus.send(&call, spend).await?;
    finished(&message)?;
    serde_json::from_str(&text_of(&message))
        .map_err(|error| LlmError::Invalid(format!("plan: {error}")))
}

pub(super) async fn verdict(
    opus: &Anthropic,
    review: &Review<'_>,
    spend: &mut Vec<Spend>,
) -> Result<Verdict, LlmError> {
    let mut content = vec![json!({"type": "text", "text": prompts::review_request(review)})];
    for picture in &review.current {
        content.push(json!({"type": "text", "text": picture.name}));
        content.push(image(picture));
    }
    let mut messages = vec![json!({"role": "user", "content": content})];
    let mut nudged = false;
    for _ in 0..MAX_TURNS {
        let call = Call {
            name: "review",
            system: Some(prompts::SYSTEM),
            messages: &messages,
            tools: Some(prompts::review_tools()),
            format: None,
            effort: None,
        };
        let message = opus.send(&call, spend).await?;
        finished(&message)?;
        let uses: Vec<&Value> = message
            .content
            .iter()
            .filter(|block| block["type"] == "tool_use")
            .collect();
        if let Some(verdict) = uses.iter().find_map(|tool| decided(tool)) {
            return verdict;
        }
        let results: Vec<Value> = uses.iter().map(|tool| viewed(review, tool)).collect();
        messages.push(json!({"role": "assistant", "content": message.content}));
        if results.is_empty() {
            if nudged {
                return Err(LlmError::Invalid(
                    "the review ended without a verdict".to_owned(),
                ));
            }
            nudged = true;
            messages.push(json!({"role": "user", "content": NUDGE}));
            continue;
        }
        messages.push(json!({"role": "user", "content": results}));
    }
    Err(LlmError::Invalid(
        "the review took too many turns".to_owned(),
    ))
}

fn decided(tool: &Value) -> Option<Result<Verdict, LlmError>> {
    let input = &tool["input"];
    match tool["name"].as_str()? {
        "accept" => Some(Ok(Verdict::Accept {
            trim_start_ms: input["trim_start_ms"]
                .as_i64()
                .map_or(0, |trim| trim.clamp(0, i64::from(i32::MAX)) as i32),
        })),
        "reject" => Some(match (input["reason"].as_str(), input["edit"].as_str()) {
            (Some(reason), Some(edit)) => Ok(Verdict::Redo {
                reason: reason.to_owned(),
                edit: edit.to_owned(),
            }),
            _ => Err(LlmError::Invalid(
                "reject without reason or edit".to_owned(),
            )),
        }),
        _ => None,
    }
}

fn viewed(review: &Review<'_>, tool: &Value) -> Value {
    let id = tool["id"].clone();
    let name = tool["input"]["name"].as_str().unwrap_or_default();
    let found = match name {
        "cover" => {
            Some(json!({"type": "image", "source": {"type": "url", "url": review.cover_url}}))
        }
        _ => name
            .strip_prefix("keyframe")
            .and_then(|number| number.parse::<usize>().ok())
            .and_then(|number| review.earlier.get(number))
            .map(image),
    };
    match found {
        Some(picture) => json!({"type": "tool_result", "tool_use_id": id, "content": [picture]}),
        None => json!({
            "type": "tool_result",
            "tool_use_id": id,
            "is_error": true,
            "content": format!("no frame {name}; there are cover and keyframe0..{}", review.earlier.len()),
        }),
    }
}

fn image(picture: &Picture) -> Value {
    json!({
        "type": "image",
        "source": {
            "type": "base64",
            "media_type": picture.media_type,
            "data": base64::engine::general_purpose::STANDARD.encode(&picture.data),
        },
    })
}
