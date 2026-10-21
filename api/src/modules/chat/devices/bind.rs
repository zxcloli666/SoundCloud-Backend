use chat_wire::bind::verify_bind;
use chat_wire::prost::Message;
use chat_wire::proto::Bind;
use uuid::Uuid;

use crate::modules::chat::{ChatError, ChatResult};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundDevice {
    pub id: Uuid,
    pub auth_pub: [u8; 32],
    pub box_pub: [u8; 32],
}

pub fn bound_device(chat_bind: Option<&[u8]>) -> ChatResult<BoundDevice> {
    let bytes = chat_bind.ok_or(ChatError::ReloginRequired)?;
    let bind = Bind::decode(bytes).map_err(|_| ChatError::ReloginRequired)?;
    verify_bind(&bind).map_err(|_| ChatError::ReloginRequired)?;
    let id = Uuid::parse_str(&bind.device_id).map_err(|_| ChatError::ReloginRequired)?;
    Ok(BoundDevice {
        id,
        auth_pub: key(&bind.auth_pub)?,
        box_pub: key(&bind.box_pub)?,
    })
}

impl BoundDevice {
    pub fn holds(&self, auth_pub: &[u8], box_pub: &[u8]) -> bool {
        self.auth_pub.as_slice() == auth_pub && self.box_pub.as_slice() == box_pub
    }
}

fn key(bytes: &[u8]) -> ChatResult<[u8; 32]> {
    <[u8; 32]>::try_from(bytes).map_err(|_| ChatError::ReloginRequired)
}
