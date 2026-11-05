use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sc_chat_wire::pow::POW_V;
use sc_chat_wire::proto::{Header, InnerResponse};
use serde_json::json;

use crate::modules::chat::ChatError;
use crate::modules::chat::channel::answer::error_answer;

pub enum Refusal {
    Chat(ChatError),
    Required { seed: [u8; 32], effort: u32 },
}

impl From<ChatError> for Refusal {
    fn from(error: ChatError) -> Self {
        Refusal::Chat(error)
    }
}

impl Refusal {
    pub fn outcome(&self) -> &'static str {
        match self {
            Refusal::Chat(error) => error.code(),
            Refusal::Required { .. } => "chat_pow_required",
        }
    }

    pub fn into_answer(self) -> InnerResponse {
        match self {
            Refusal::Chat(error) => error_answer(error),
            Refusal::Required { seed, effort } => InnerResponse {
                status: u32::from(StatusCode::TOO_MANY_REQUESTS.as_u16()),
                headers: vec![Header {
                    name: CONTENT_TYPE.as_str().to_owned(),
                    value: "application/json".to_owned(),
                }],
                body: json!({
                    "error": "chat_pow_required",
                    "v": POW_V,
                    "seed": URL_SAFE_NO_PAD.encode(seed),
                    "effort": effort,
                })
                .to_string()
                .into_bytes(),
            },
        }
    }
}
