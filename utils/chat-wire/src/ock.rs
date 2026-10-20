use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable, Kem, Serializable};

use crate::WireError;

pub const OCK_PINS: &[(u8, [u8; 32])] = &[
    (
        1,
        hex32("46b0bccd368d8553a5e09bc8aff285142c58be72454504d01dd7cd964991ee63"),
    ),
    (
        2,
        hex32("a4ba96381c8a03bf977799af43ae573eba75d787a47acb7cc2b6622595d43054"),
    ),
];

pub fn pinned(version: u8) -> Option<&'static [u8; 32]> {
    OCK_PINS
        .iter()
        .find(|(pinned, _)| *pinned == version)
        .map(|(_, key)| key)
}

pub struct OckSecret(<X25519HkdfSha256 as Kem>::PrivateKey);

impl OckSecret {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, WireError> {
        if bytes.len() != 32 {
            return Err(WireError::Key);
        }
        <X25519HkdfSha256 as Kem>::PrivateKey::from_bytes(bytes)
            .map(Self)
            .map_err(|_| WireError::Key)
    }

    pub fn public(&self) -> [u8; 32] {
        X25519HkdfSha256::sk_to_pk(&self.0).to_bytes().into()
    }

    pub(crate) fn key(&self) -> &<X25519HkdfSha256 as Kem>::PrivateKey {
        &self.0
    }
}

const fn hex32(text: &str) -> [u8; 32] {
    let digits = text.as_bytes();
    assert!(digits.len() == 64, "a pin is 64 hex digits");
    let mut key = [0u8; 32];
    let mut at = 0;
    while at < 32 {
        key[at] = (nibble(digits[2 * at]) << 4) | nibble(digits[2 * at + 1]);
        at += 1;
    }
    key
}

const fn nibble(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'a'..=b'f' => digit - b'a' + 10,
        _ => panic!("a pin is lowercase hex"),
    }
}
