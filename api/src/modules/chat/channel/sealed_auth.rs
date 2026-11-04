use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use sc_chat_wire::bind::verify_bind;
use sc_chat_wire::prost::Message;
use sc_chat_wire::proto::Inner;
use sc_chat_wire::sign::verify_request;
use uuid::Uuid;

use crate::modules::chat::ChatError;
use crate::modules::chat::caller::CallerBind;

#[derive(Clone, Debug)]
pub struct SealedAuth {
    pub bind: Vec<u8>,
    pub keys: CallerBind,
    pub session_id: Option<Uuid>,
}

impl SealedAuth {
    pub fn verify(inner: &Inner, enc: &[u8; 32]) -> Result<Self, ChatError> {
        let bind = inner.bind.as_ref().ok_or(ChatError::Invalid)?;
        verify_bind(bind).map_err(|_| ChatError::Invalid)?;
        let keys = CallerBind::from_bind(bind).ok_or(ChatError::Invalid)?;
        verify_request(&bind.auth_pub, inner, enc).map_err(|_| ChatError::Invalid)?;
        let session_id = inner
            .session_id
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|_| ChatError::Invalid)?;
        Ok(Self {
            bind: bind.encode_to_vec(),
            keys,
            session_id,
        })
    }
}

impl<S: Send + Sync> FromRequestParts<S> for SealedAuth {
    type Rejection = ChatError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<SealedAuth>()
            .cloned()
            .ok_or(ChatError::Internal)
    }
}
