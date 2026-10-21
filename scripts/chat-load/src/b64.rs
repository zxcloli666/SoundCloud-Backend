use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};

pub fn encode(bytes: &[u8]) -> String {
    STANDARD_NO_PAD.encode(bytes)
}

pub fn padded(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub fn decode(text: &str) -> Option<Vec<u8>> {
    STANDARD_NO_PAD
        .decode(text)
        .or_else(|_| STANDARD.decode(text))
        .ok()
}
