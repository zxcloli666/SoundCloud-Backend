use crate::WireError;
use crate::proto::Bind;
use crate::sign::{public_key, sign, verify};

pub fn bind_string(device_id: &str, auth_pub: &[u8], box_pub: &[u8], ts_ms: u64) -> String {
    format!(
        "scd-chat/bind/v1\n{device_id}\n{}\n{}\n{ts_ms}",
        hex::encode(auth_pub),
        hex::encode(box_pub),
    )
}

pub fn make_bind(device_id: &str, auth_seed: &[u8; 32], box_pub: &[u8; 32], ts_ms: u64) -> Bind {
    let auth_pub = public_key(auth_seed);
    let signed = bind_string(device_id, &auth_pub, box_pub, ts_ms);
    Bind {
        device_id: device_id.to_owned(),
        auth_pub: auth_pub.to_vec(),
        box_pub: box_pub.to_vec(),
        ts_ms,
        sig: sign(auth_seed, &signed).to_vec(),
    }
}

pub fn verify_bind(bind: &Bind) -> Result<(), WireError> {
    if bind.auth_pub.len() != 32 || bind.box_pub.len() != 32 || bind.device_id.contains('\n') {
        return Err(WireError::Key);
    }
    let signed = bind_string(&bind.device_id, &bind.auth_pub, &bind.box_pub, bind.ts_ms);
    verify(&bind.auth_pub, &signed, &bind.sig)
}
