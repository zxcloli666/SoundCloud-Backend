use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

pub const BLOCK: usize = 4096;
pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;
pub const PAD_MARK: u8 = 0x80;
const AAD_TAG: &str = "chat-store/media/v1\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Image,
    Thumb,
}

impl Role {
    fn label(self) -> &'static [u8] {
        match self {
            Role::Image => b"image",
            Role::Thumb => b"thumb",
        }
    }
}

pub fn aad(role: Role) -> Vec<u8> {
    let label = role.label();
    let mut aad = AAD_TAG.as_bytes().to_vec();
    aad.extend_from_slice(&(label.len() as u32).to_be_bytes());
    aad.extend_from_slice(label);
    aad
}

pub fn open(role: Role, key: &[u8], nonce: &[u8], ciphertext: &[u8]) -> Option<Vec<u8>> {
    let key: &[u8; KEY_LEN] = key.try_into().ok()?;
    if nonce.len() != NONCE_LEN || ciphertext.is_empty() || !ciphertext.len().is_multiple_of(BLOCK)
    {
        return None;
    }
    let mut plain = XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: &aad(role),
            },
        )
        .ok()?;
    let end = plain
        .iter()
        .rposition(|byte| *byte != 0)
        .filter(|at| plain[*at] == PAD_MARK)?;
    plain.truncate(end);
    Some(plain)
}

#[cfg(test)]
#[path = "../tests/admin_cipher_tests.rs"]
mod tests;
