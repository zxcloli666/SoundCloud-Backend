use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chacha20poly1305::aead::{Aead, KeyInit, OsRng, Payload};
use chacha20poly1305::{AeadCore, XChaCha20Poly1305, XNonce};

const PREFIX: &str = "v1:";
const NONCE_BYTES: usize = 24;

#[derive(Clone)]
pub(crate) struct Seal {
    cipher: XChaCha20Poly1305,
}

impl std::fmt::Debug for Seal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Seal")
    }
}

impl Seal {
    pub(crate) fn new(key: &[u8; 32]) -> Self {
        Self {
            cipher: XChaCha20Poly1305::new(key.into()),
        }
    }

    pub(crate) fn seal(&self, name: &str, plain: &str) -> anyhow::Result<String> {
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let sealed = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plain.as_bytes(),
                    aad: name.as_bytes(),
                },
            )
            .map_err(|_| anyhow::anyhow!("account secret was not sealed"))?;
        let mut packed = nonce.to_vec();
        packed.extend_from_slice(&sealed);
        Ok(format!("{PREFIX}{}", STANDARD.encode(packed)))
    }

    pub(crate) fn open(&self, name: &str, stored: &str) -> anyhow::Result<String> {
        let broken = || anyhow::anyhow!("account {name} secret does not open");
        let packed = stored
            .strip_prefix(PREFIX)
            .and_then(|body| STANDARD.decode(body).ok())
            .filter(|packed| packed.len() > NONCE_BYTES)
            .ok_or_else(broken)?;
        let (nonce, sealed) = packed.split_at(NONCE_BYTES);
        let plain = self
            .cipher
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: sealed,
                    aad: name.as_bytes(),
                },
            )
            .map_err(|_| broken())?;
        String::from_utf8(plain).map_err(|_| broken())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_opens_only_under_its_own_name_and_key() {
        let seal = Seal::new(&[3; 32]);

        let stored = seal.seal("yt1", "cookie text").unwrap();

        assert!(stored.starts_with(PREFIX));
        assert!(!stored.contains("cookie"));
        assert_eq!(seal.open("yt1", &stored).unwrap(), "cookie text");
        assert!(seal.open("yt2", &stored).is_err());
        assert!(Seal::new(&[4; 32]).open("yt1", &stored).is_err());
        assert!(seal.open("yt1", "cookie text").is_err());
    }

    #[test]
    fn two_seals_of_one_secret_differ() {
        let seal = Seal::new(&[3; 32]);

        assert_ne!(
            seal.seal("yt1", "same").unwrap(),
            seal.seal("yt1", "same").unwrap()
        );
    }
}
