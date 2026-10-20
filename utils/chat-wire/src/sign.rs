use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::WireError;
use crate::proto::Inner;

pub fn public_key(seed: &[u8; 32]) -> [u8; 32] {
    SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

pub fn sign(seed: &[u8; 32], message: &str) -> [u8; 64] {
    SigningKey::from_bytes(seed)
        .sign(message.as_bytes())
        .to_bytes()
}

pub fn verify(public: &[u8], message: &str, signature: &[u8]) -> Result<(), WireError> {
    let public: &[u8; 32] = public.try_into().map_err(|_| WireError::Key)?;
    let key = VerifyingKey::from_bytes(public).map_err(|_| WireError::Key)?;
    let signature = Signature::from_slice(signature).map_err(|_| WireError::Signature)?;
    key.verify_strict(message.as_bytes(), &signature)
        .map_err(|_| WireError::Signature)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn request_string(inner: &Inner, enc: &[u8; 32]) -> String {
    format!(
        "scd-chat/req/v2\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        inner.method,
        inner.path,
        inner.query,
        inner.ts_ms,
        sha256_hex(&inner.body),
        inner.session_id.as_deref().unwrap_or("-"),
        inner.device_id.as_deref().unwrap_or("-"),
        sha256_hex(enc),
    )
}

pub fn sign_request(seed: &[u8; 32], inner: &mut Inner, enc: &[u8; 32]) {
    inner.sig = sign(seed, &request_string(inner, enc)).to_vec();
}

pub fn verify_request(public: &[u8], inner: &Inner, enc: &[u8; 32]) -> Result<(), WireError> {
    let fields = [
        inner.method.as_str(),
        inner.path.as_str(),
        inner.query.as_str(),
        inner.session_id.as_deref().unwrap_or(""),
        inner.device_id.as_deref().unwrap_or(""),
    ];
    if fields.iter().any(|field| field.contains('\n')) {
        return Err(WireError::Signature);
    }
    verify(public, &request_string(inner, enc), &inner.sig)
}
