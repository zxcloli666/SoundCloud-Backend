use axum::body::Body;
use axum::http::{Extensions, Request};
use chat_wire::prost::Message;
use chat_wire::proto::{Inner, InnerResponse};
use chat_wire::seal::OpenedRequest;

use crate::modules::chat::caller::{Resolved, resolve};
use crate::modules::chat::channel::answer::error_answer;
use crate::modules::chat::channel::dispatch::{dispatch, inner_request};
use crate::modules::chat::channel::sealed_auth::SealedAuth;
use crate::modules::chat::channel::{
    CONFIG_PATH, Channel, DEVICES_PATH, clock, is_sealed_auth, replay,
};
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub async fn answer(channel: &Channel, opened: &OpenedRequest) -> InnerResponse {
    match prepare(channel, opened).await {
        Ok(request) => dispatch(channel, request).await,
        Err(error) => error_answer(error),
    }
}

async fn prepare(channel: &Channel, opened: &OpenedRequest) -> ChatResult<Request<Body>> {
    let inner = Inner::decode(opened.inner.as_slice()).map_err(|_| ChatError::Invalid)?;
    let chat = &channel.chat;
    let sealed_auth = is_sealed_auth(&inner.path);
    if !sealed_auth && !chat.config.enabled {
        return Err(ChatError::Disabled);
    }
    clock::check(inner.ts_ms)?;
    let mut extensions = Extensions::new();
    if sealed_auth {
        let auth = SealedAuth::verify(&inner, &opened.enc)?;
        channel.gate.admit(&inner.path)?;
        replay::claim(chat, &opened.enc).await?;
        extensions.insert(auth);
    } else {
        if !inner.path.starts_with("/chat/") {
            return Err(ChatError::NotFound);
        }
        let resolved = resolve(chat, &inner, &opened.enc).await?;
        replay::claim(chat, &opened.enc).await?;
        admit(chat, &inner.method, &inner.path, &resolved)?;
        extensions.insert(resolved.caller);
        if let Some(bind) = resolved.bind {
            extensions.insert(bind);
        }
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
