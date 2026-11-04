use sha2::{Digest, Sha256};

use super::{BLOCK, Role, aad, open};
use crate::modules::chat::admin::media_support::{seal_blob, sealed_len};

fn key() -> [u8; 32] {
    std::array::from_fn(|at| at as u8)
}

fn nonce() -> [u8; 24] {
    std::array::from_fn(|at| 100 + at as u8)
}

struct Vector {
    role: Role,
    len: usize,
    zero_tail: bool,
    sealed: usize,
    sha256: &'static str,
    tag: &'static str,
}

const PYNACL_VECTORS: [Vector; 3] = [
    Vector {
        role: Role::Image,
        len: 37,
        zero_tail: false,
        sealed: 4096,
        sha256: "dbc4f7de510ba0e5b4e622e0a5158661c220ea7d1c1ad3e112d5c4a6816b96c1",
        tag: "74c3e697524cd6992f0de59da3c5d8d5",
    },
    Vector {
        role: Role::Thumb,
        len: 4079,
        zero_tail: true,
        sealed: 4096,
        sha256: "f86673a9577361ed9508e8b23ec0dfa4261040e92a72d0141aac274d54ffbccb",
        tag: "39ccdc09f4730317bfd91aabf0b674aa",
    },
    Vector {
        role: Role::Image,
        len: 4080,
        zero_tail: false,
        sealed: 8192,
        sha256: "dcf3cc7e65c3b2ab85f74e62030d28fa7e62f27f87a95a9f9b28174162d4c964",
        tag: "e9eaa9888c4690615903a17dd9b128e2",
    },
];

fn plain(vector: &Vector) -> Vec<u8> {
    let mut plain: Vec<u8> = (0..vector.len)
        .map(|at| ((at * 7 + 3) % 256) as u8)
        .collect();
    if vector.zero_tail {
        let end = plain.len();
        plain[end - 2..].fill(0);
    }
    plain
}

#[test]
fn the_aad_is_the_tagged_core_label() {
    assert_eq!(
        aad(Role::Image),
        b"chat-store/media/v1\n\0\0\0\x05image".to_vec()
    );
    assert_eq!(
        aad(Role::Thumb),
        b"chat-store/media/v1\n\0\0\0\x05thumb".to_vec()
    );
}

#[test]
fn blobs_sealed_by_an_independent_xchacha_implementation_open_byte_for_byte() {
    for vector in &PYNACL_VECTORS {
        let plain = plain(vector);
        let sealed = seal_blob(vector.role, &key(), &nonce(), &plain);
        assert_eq!(sealed.len(), vector.sealed);
        assert_eq!(sealed.len(), sealed_len(vector.len));
        assert_eq!(hex::encode(Sha256::digest(&sealed)), vector.sha256);
        assert_eq!(hex::encode(&sealed[sealed.len() - 16..]), vector.tag);
        assert_eq!(
            open(vector.role, &key(), &nonce(), &sealed),
            Some(plain),
            "{} bytes",
            vector.len
        );
    }
}

#[test]
fn a_blob_opens_only_with_its_role_key_nonce_and_whole_blocks() {
    let plain = b"RIFF\0\0\0\0WEBPVP8L tiny".to_vec();
    let sealed = seal_blob(Role::Image, &key(), &nonce(), &plain);
    assert_eq!(open(Role::Thumb, &key(), &nonce(), &sealed), None);
    let mut other_key = key();
    other_key[0] ^= 1;
    assert_eq!(open(Role::Image, &other_key, &nonce(), &sealed), None);
    assert_eq!(open(Role::Image, &key()[..31], &nonce(), &sealed), None);
    assert_eq!(open(Role::Image, &key(), &nonce()[..12], &sealed), None);
    let mut flipped = sealed.clone();
    flipped[10] ^= 0x40;
    assert_eq!(open(Role::Image, &key(), &nonce(), &flipped), None);
    assert_eq!(
        open(Role::Image, &key(), &nonce(), &sealed[..BLOCK - 1]),
        None
    );
    assert_eq!(open(Role::Image, &key(), &nonce(), &[]), None);
    assert_eq!(open(Role::Image, &key(), &nonce(), &sealed), Some(plain));
}

#[test]
fn padding_without_the_mark_is_refused() {
    let unmarked = vec![0x11u8; BLOCK - 16];
    let sealed = chacha_seal(&unmarked);
    assert_eq!(open(Role::Image, &key(), &nonce(), &sealed), None);
    let zeros = vec![0u8; BLOCK - 16];
    assert_eq!(
        open(Role::Image, &key(), &nonce(), &chacha_seal(&zeros)),
        None
    );
    let mut marked = vec![0u8; BLOCK - 16];
    marked[0] = 0x80;
    assert_eq!(
        open(Role::Image, &key(), &nonce(), &chacha_seal(&marked)),
        Some(Vec::new())
    );
}

fn chacha_seal(padded: &[u8]) -> Vec<u8> {
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    use chacha20poly1305::{XChaCha20Poly1305, XNonce};
    XChaCha20Poly1305::new((&key()).into())
        .encrypt(
            XNonce::from_slice(&nonce()),
            Payload {
                msg: padded,
                aad: &aad(Role::Image),
            },
        )
        .expect("a sealed block")
}
