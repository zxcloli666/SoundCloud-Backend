use hmac::{Hmac, Mac};
use sha1::Sha1;

const STEP_SECONDS: u64 = 30;
const DIGITS: u32 = 1_000_000;

pub(super) fn code(key: &[u8], unix_seconds: u64) -> Option<String> {
    let counter = (unix_seconds / STEP_SECONDS).to_be_bytes();
    let mut mac = Hmac::<Sha1>::new_from_slice(key).ok()?;
    mac.update(&counter);
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest.last().copied()? & 0x0f);
    let window: [u8; 4] = digest.get(offset..offset + 4)?.try_into().ok()?;
    let truncated = u32::from_be_bytes(window) & 0x7fff_ffff;
    Some(format!("{:06}", truncated % DIGITS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_rfc_6238_vectors() {
        let key = b"12345678901234567890";

        assert_eq!(code(key, 59).as_deref(), Some("287082"));
        assert_eq!(code(key, 1_111_111_109).as_deref(), Some("081804"));
        assert_eq!(code(key, 1_234_567_890).as_deref(), Some("005924"));
    }
}
