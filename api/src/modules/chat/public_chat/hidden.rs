use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use crate::modules::chat::public_chat::ID;
use crate::modules::chat::{ChatCaller, ChatError, ChatResult};

const CONVERSATIONS: &str = "/chat/conversations/";
const PUBLIC: &str = "/chat/public/";

pub fn from(caller: &ChatCaller, conversation: Uuid) -> ChatResult<()> {
    if conversation == ID && !caller.sees_public() {
        return Err(ChatError::NotFound);
    }
    Ok(())
}

pub async fn gate(request: Request, next: Next) -> Response {
    let hidden = request
        .extensions()
        .get::<ChatCaller>()
        .is_some_and(|caller| !caller.sees_public())
        && names_public(request.uri().path());
    if hidden {
        return ChatError::NotFound.into_response();
    }
    next.run(request).await
}

fn names_public(path: &str) -> bool {
    if path.starts_with(PUBLIC) {
        return true;
    }
    let Some(rest) = path.strip_prefix(CONVERSATIONS) else {
        return false;
    };
    let id = rest.split('/').next().unwrap_or_default();
    Uuid::parse_str(id).is_ok_and(|id| id == ID)
}
