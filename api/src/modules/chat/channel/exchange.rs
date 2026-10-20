use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::response::Response;
use chat_wire::prost::Message;
use chat_wire::proto::Inner;
use chat_wire::seal::{ock_version, open_request};

use crate::modules::chat::ChatError;
use crate::modules::chat::channel::answer::{bad_envelope, error_answer, sealed};
use crate::modules::chat::channel::dispatch::dispatch;
use crate::modules::chat::channel::{Channel, is_sealed_auth};

pub async fn exchange(State(channel): State<Arc<Channel>>, body: Bytes) -> Response {
    let Ok(version) = ock_version(&body) else {
        return bad_envelope();
    };
    let Some(secret) = channel.chat.ocks.get(version) else {
        return bad_envelope();
    };
    let Ok(opened) = open_request(&body, secret) else {
        return bad_envelope();
    };
    let answer = match Inner::decode(opened.inner.as_slice()) {
        Err(_) => error_answer(ChatError::Invalid),
        Ok(inner) if !channel.chat.config.enabled && !is_sealed_auth(&inner.path) => {
            error_answer(ChatError::Disabled)
        }
        Ok(inner) => dispatch(&channel, &opened, inner).await,
    };
    sealed(&opened, answer)
}
