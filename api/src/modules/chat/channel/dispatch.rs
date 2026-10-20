use chat_wire::proto::{Inner, InnerResponse};
use chat_wire::seal::OpenedRequest;

use crate::modules::chat::ChatError;
use crate::modules::chat::channel::Channel;
use crate::modules::chat::channel::answer::error_answer;

pub async fn dispatch(_channel: &Channel, _opened: &OpenedRequest, _inner: Inner) -> InnerResponse {
    error_answer(ChatError::Unimplemented)
}
