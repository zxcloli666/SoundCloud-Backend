use backend_contracts::pipeline::{RpcError, SayData};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use serde::Deserialize;
use serde_json::Value;

const OGG_MAGIC: &[u8] = b"OggS";

#[derive(Debug, PartialEq)]
pub enum Heard {
    Clip { clip: Bytes, seconds: f64 },
    Failed(&'static str),
}

#[derive(Deserialize)]
struct RpcReply {
    ok: bool,
    data: Option<SayData>,
    error: Option<String>,
}

pub fn heard(reply: &[u8]) -> Heard {
    let Ok(reply) = serde_json::from_slice::<RpcReply>(reply) else {
        return Heard::Failed("malformed");
    };
    match (reply.ok, reply.data) {
        (true, Some(data)) => {
            ogg(&data.clip).map_or(Heard::Failed("malformed"), |clip| Heard::Clip {
                clip,
                seconds: data.seconds,
            })
        }
        (true, None) => Heard::Failed("malformed"),
        (false, _) => Heard::Failed(refusal(reply.error.unwrap_or_default())),
    }
}

fn ogg(encoded: &str) -> Option<Bytes> {
    let clip = STANDARD.decode(encoded).ok()?;
    clip.starts_with(OGG_MAGIC).then(|| Bytes::from(clip))
}

fn refusal(error: String) -> &'static str {
    match serde_json::from_value(Value::String(error)) {
        Ok(RpcError::Expired) => "expired",
        Ok(RpcError::InvalidRequest) => "invalid",
        Ok(RpcError::Internal) => "internal",
        Err(_) => "rejected",
    }
}
