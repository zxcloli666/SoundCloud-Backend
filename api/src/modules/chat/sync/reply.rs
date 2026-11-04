use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rand::Rng;

use crate::modules::chat::sync::wire::{RETRY_AFTER_MS, SyncAnswer};

pub const LATER_MAX_MS: u64 = 10_000;

#[derive(Debug)]
pub enum SyncReply {
    Answer(Box<SyncAnswer>),
    Idle,
    Superseded,
    Later(u64),
}

impl SyncReply {
    pub fn later() -> Self {
        SyncReply::Later(rand::thread_rng().gen_range(0..=LATER_MAX_MS))
    }

    pub fn reason(&self) -> &'static str {
        match self {
            SyncReply::Answer(answer) if answer.reset => "reset",
            SyncReply::Answer(_) => "changes",
            SyncReply::Idle => "idle",
            SyncReply::Superseded => "superseded",
            SyncReply::Later(_) => "later",
        }
    }
}

impl IntoResponse for SyncReply {
    fn into_response(self) -> Response {
        match self {
            SyncReply::Answer(answer) => Json(answer).into_response(),
            SyncReply::Idle | SyncReply::Superseded => StatusCode::NO_CONTENT.into_response(),
            SyncReply::Later(retry_ms) => (
                StatusCode::NO_CONTENT,
                [(RETRY_AFTER_MS, retry_ms.to_string())],
            )
                .into_response(),
        }
    }
}
