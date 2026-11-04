use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::{Rng, RngCore};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use backend_contracts::remote::{LINK_CODE_DIGITS, LINK_HINT_DIGITS, LinkProvider};

pub const MIN_REVIEW_CODE_DIGITS: usize = 8;

pub fn random_code() -> String {
    let upper = 10_u32.pow(LINK_CODE_DIGITS as u32);
    format!(
        "{:0width$}",
        rand::rngs::OsRng.gen_range(0..upper),
        width = LINK_CODE_DIGITS
    )
}

pub fn random_hint() -> String {
    let lower = 10_u32.pow(LINK_HINT_DIGITS as u32 - 1);
    let upper = 10_u32.pow(LINK_HINT_DIGITS as u32);
    rand::rngs::OsRng.gen_range(lower..upper).to_string()
}

fn digits_of(raw: &str) -> String {
    raw.chars().filter(char::is_ascii_digit).collect()
}

pub fn normalize_code(raw: &str) -> Option<String> {
    let digits = digits_of(raw);
    (digits.len() == LINK_CODE_DIGITS).then_some(digits)
}

pub fn valid_review_code(code: &str) -> bool {
    code.len() >= MIN_REVIEW_CODE_DIGITS && code.chars().all(|c| c.is_ascii_digit())
}

pub fn review_code_matches(raw: &str, review_code: &str) -> bool {
    valid_review_code(review_code) && token_matches(&digits_of(raw), review_code)
}

pub fn code_hash(provider: LinkProvider, code: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"voice-link-code:");
    hash.update(provider.as_str().as_bytes());
    hash.update(b":");
    hash.update(code.as_bytes());
    hex::encode(hash.finalize())
}

pub fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn random_seed() -> String {
    random_token()
}

pub fn rotated_tokens(seed: &str, previous_refresh: &str) -> (String, String) {
    let derive = |label: &str| {
        let mut hash = Sha256::new();
        hash.update(b"voice-rotation:");
        hash.update(label.as_bytes());
        hash.update(b":");
        hash.update(seed.as_bytes());
        hash.update(b":");
        hash.update(previous_refresh.as_bytes());
        URL_SAFE_NO_PAD.encode(hash.finalize())
    };
    (derive("access"), derive("refresh"))
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn secret_matches(provided: &str, expected_sha256: &[u8]) -> bool {
    let provided = Sha256::digest(provided.as_bytes());
    provided.as_slice().ct_eq(expected_sha256).into()
}

pub fn token_matches(provided: &str, expected: &str) -> bool {
    if expected.is_empty() {
        return false;
    }
    let provided = Sha256::digest(provided.as_bytes());
    let expected = Sha256::digest(expected.as_bytes());
    provided.as_slice().ct_eq(expected.as_slice()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_is_six_digits_with_leading_zeros_kept() {
        for _ in 0..200 {
            let code = random_code();
            assert_eq!(code.len(), 6);
            assert!(code.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn a_hint_is_two_digits_without_a_leading_zero() {
        for _ in 0..500 {
            let hint = random_hint();
            assert_eq!(hint.len(), 2, "{hint}");
            assert!(hint.chars().all(|c| c.is_ascii_digit()));
            assert!(!hint.starts_with('0'));
        }
    }

    #[test]
    fn the_review_code_needs_eight_digits_and_never_matches_a_link_code() {
        assert!(review_code_matches("12345678", "12345678"));
        assert!(review_code_matches("1234 5678", "12345678"));
        assert!(review_code_matches("код 1 2 3 4 5 6 7 8", "12345678"));
        assert!(!review_code_matches("123456", "12345678"));
        assert!(!review_code_matches("12345679", "12345678"));
        assert!(!review_code_matches("123456", "123456"));
        assert!(!review_code_matches("", ""));
        assert!(!review_code_matches("abcdefgh", "abcdefgh"));
        assert!(valid_review_code("00000000"));
        assert!(!valid_review_code("1234567"));
        assert!(!valid_review_code("1234567a"));
    }

    #[test]
    fn spoken_and_typed_codes_normalize() {
        assert_eq!(normalize_code("123 456").as_deref(), Some("123456"));
        assert_eq!(normalize_code("123-456").as_deref(), Some("123456"));
        assert_eq!(normalize_code("12345"), None);
        assert_eq!(normalize_code("1234567"), None);
        assert_eq!(normalize_code("код"), None);
    }

    #[test]
    fn a_code_hash_depends_on_the_provider() {
        let alice = code_hash(LinkProvider::Alice, "123456");
        assert_eq!(alice, code_hash(LinkProvider::Alice, "123456"));
        assert_ne!(alice, code_hash(LinkProvider::Salut, "123456"));
        assert_ne!(alice, code_hash(LinkProvider::Alice, "123457"));
    }

    #[test]
    fn tokens_are_long_unique_and_hash_to_hex() {
        let first = random_token();
        assert!(first.len() >= 43);
        assert_ne!(first, random_token());
        assert_eq!(token_hash(&first).len(), 64);
        assert_ne!(token_hash(&first), first);
    }

    #[test]
    fn a_rotation_repeats_the_same_pair_only_for_the_same_seed_and_token() {
        let first = rotated_tokens("seed", "old-refresh");
        assert_eq!(first, rotated_tokens("seed", "old-refresh"));
        assert_ne!(first, rotated_tokens("other", "old-refresh"));
        assert_ne!(first, rotated_tokens("seed", "another-refresh"));
        assert_ne!(first.0, first.1);
        assert!(first.0.len() >= 43);
    }

    #[test]
    fn secrets_compare_by_digest() {
        let expected = Sha256::digest(b"right");
        assert!(secret_matches("right", expected.as_slice()));
        assert!(!secret_matches("wrong", expected.as_slice()));
        assert!(!secret_matches("right", &[0_u8; 4]));
    }

    #[test]
    fn an_empty_shared_token_never_matches() {
        assert!(!token_matches("", ""));
        assert!(!token_matches("x", ""));
        assert!(token_matches("x", "x"));
        assert!(!token_matches("x", "y"));
    }
}
