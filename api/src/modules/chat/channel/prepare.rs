use axum::body::Body;
use axum::http::{Extensions, Request};
use sc_chat_wire::prost::Message;
use sc_chat_wire::proto::{Inner, InnerResponse};
use sc_chat_wire::seal::OpenedRequest;

use crate::modules::chat::caller::{Resolved, resolve};
use crate::modules::chat::channel::answer::error_answer;
use crate::modules::chat::channel::dispatch::{dispatch, inner_request};
use crate::modules::chat::channel::{CONFIG_PATH, Channel, DEVICES_PATH, clock, replay};
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub async fn answer(channel: &Channel, opened: &OpenedRequest) -> InnerResponse {
    match prepare(channel, opened).await {
        Ok(request) => dispatch(&channel.inner, request).await,
        Err(error) => error_answer(error),
    }
}

async fn prepare(channel: &Channel, opened: &OpenedRequest) -> ChatResult<Request<Body>> {
    let inner = Inner::decode(opened.inner.as_slice()).map_err(|_| ChatError::Invalid)?;
    let chat = &channel.chat;
    if !chat.config.enabled {
        return Err(ChatError::Disabled);
    }
    clock::check(inner.ts_ms)?;
    if !inner.path.starts_with("/chat/") {
        return Err(ChatError::NotFound);
    }
    let mut extensions = Extensions::new();
    let resolved = resolve(chat, &inner, &opened.enc).await?;
    replay::claim(chat, &opened.enc).await?;
    admit(chat, &inner.method, &inner.path, &resolved)?;
    extensions.insert(resolved.caller);
    if let Some(bind) = resolved.bind {
        extensions.insert(bind);
    }
    inner_request(inner, extensions)
}

fn admit(chat: &ChatState, method: &str, path: &str, resolved: &Resolved) -> ChatResult<()> {
    if path == CONFIG_PATH {
        return Ok(());
    }
    if resolved.bind.is_none() {
        return Err(ChatError::ReloginRequired);
    }
    if !chat.config.opens_for(&resolved.caller.user_id) {
        return Err(ChatError::Disabled);
    }
    if resolved.caller.device.is_none() && !registers(method, path) {
        return Err(ChatError::DeviceRevoked);
    }
    Ok(())
}

fn registers(method: &str, path: &str) -> bool {
    path == DEVICES_PATH && matches!(method, "GET" | "POST")
}
