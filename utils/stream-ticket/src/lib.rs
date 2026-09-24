use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use thiserror::Error;
use uuid::Uuid;

const VERSION: u8 = 1;
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const HEADER_LEN: usize = 1 + 1 + 8 + 16 + 2;
const MAX_SECRET_LEN: usize = 1024;
const MAX_TICKET_LEN: usize = 2048;
const MAX_KEYS: usize = 4;
const MAX_LIFETIME: Duration = Duration::from_secs(300);
const AAD_PREFIX: &[u8] = b"scd-stream-ticket-v1\0";
const HIGH_QUALITY_FLAG: u8 = 1;

#[derive(Clone)]
pub struct StreamTicketKey(LessSafeKey);

#[derive(Clone, Debug)]
pub struct StreamTicketKeys(Vec<StreamTicketKey>);

#[derive(Clone, PartialEq, Eq)]
pub struct StreamTicket {
    pub session_id: Uuid,
    pub secret_token: Option<String>,
    pub high_quality: bool,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum StreamTicketError {
    #[error("stream ticket key must contain exactly 32 bytes")]
    InvalidKey,
    #[error("stream ticket key ring must contain between 1 and 4 keys")]
    InvalidKeyRing,
    #[error("stream ticket secret is too long")]
    SecretTooLong,
    #[error("system clock is before the Unix epoch")]
    InvalidClock,
    #[error("failed to generate a stream ticket nonce")]
    RandomUnavailable,
    #[error("stream ticket is invalid")]
    InvalidTicket,
    #[error("stream ticket has expired")]
    Expired,
}

impl StreamTicketKey {
    pub fn from_base64(encoded: &str) -> Result<Self, StreamTicketError> {
        let encoded = encoded.trim();
        let bytes = STANDARD
            .decode(encoded)
            .or_else(|_| URL_SAFE_NO_PAD.decode(encoded))
            .map_err(|_| StreamTicketError::InvalidKey)?;
        let key: [u8; KEY_LEN] = bytes
            .try_into()
            .map_err(|_| StreamTicketError::InvalidKey)?;
        let key = UnboundKey::new(&aead::CHACHA20_POLY1305, &key)
            .map_err(|_| StreamTicketError::InvalidKey)?;
        Ok(Self(LessSafeKey::new(key)))
    }

    pub fn issue(
        &self,
        session_id: Uuid,
        track_urn: &str,
        secret_token: Option<&str>,
        high_quality: bool,
        ttl: Duration,
    ) -> Result<String, StreamTicketError> {
        self.issue_at(
            session_id,
            track_urn,
            secret_token,
            high_quality,
            ttl,
            SystemTime::now(),
        )
    }

    pub fn verify(
        &self,
        encoded: &str,
        track_urn: &str,
    ) -> Result<StreamTicket, StreamTicketError> {
        self.verify_at(encoded, track_urn, SystemTime::now())
    }

    fn issue_at(
        &self,
        session_id: Uuid,
        track_urn: &str,
        secret_token: Option<&str>,
        high_quality: bool,
        ttl: Duration,
        now: SystemTime,
    ) -> Result<String, StreamTicketError> {
        let ttl = ttl.min(MAX_LIFETIME);
        let expires_at = unix_seconds(now)?
            .checked_add(ttl.as_secs())
            .ok_or(StreamTicketError::InvalidClock)?;
        let secret = secret_token.unwrap_or_default().as_bytes();
        let secret_len: u16 = secret
            .len()
            .try_into()
            .map_err(|_| StreamTicketError::SecretTooLong)?;
        if secret.len() > MAX_SECRET_LEN {
            return Err(StreamTicketError::SecretTooLong);
        }

        let mut plaintext = Vec::with_capacity(HEADER_LEN + secret.len() + TAG_LEN);
        let flags = u8::from(high_quality) * HIGH_QUALITY_FLAG;
        plaintext.push(VERSION);
        plaintext.push(flags);
        plaintext.extend_from_slice(&expires_at.to_be_bytes());
        plaintext.extend_from_slice(session_id.as_bytes());
        plaintext.extend_from_slice(&secret_len.to_be_bytes());
        plaintext.extend_from_slice(secret);

        let mut nonce_bytes = [0_u8; NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce_bytes)
            .map_err(|_| StreamTicketError::RandomUnavailable)?;
        self.0
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce_bytes),
                Aad::from(aad(track_urn)),
                &mut plaintext,
            )
            .map_err(|_| StreamTicketError::InvalidTicket)?;

        let mut ticket = Vec::with_capacity(NONCE_LEN + plaintext.len());
        ticket.extend_from_slice(&nonce_bytes);
        ticket.extend_from_slice(&plaintext);
        Ok(URL_SAFE_NO_PAD.encode(ticket))
    }

    fn verify_at(
        &self,
        encoded: &str,
        track_urn: &str,
        now: SystemTime,
    ) -> Result<StreamTicket, StreamTicketError> {
        if encoded.len() > MAX_TICKET_LEN {
            return Err(StreamTicketError::InvalidTicket);
        }
        let ticket = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| StreamTicketError::InvalidTicket)?;
        if ticket.len() < NONCE_LEN + HEADER_LEN + TAG_LEN {
            return Err(StreamTicketError::InvalidTicket);
        }

        let (nonce, encrypted) = ticket.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce
            .try_into()
            .map_err(|_| StreamTicketError::InvalidTicket)?;
        let mut plaintext = encrypted.to_vec();
        let plaintext = self
            .0
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(aad(track_urn)),
                &mut plaintext,
            )
            .map_err(|_| StreamTicketError::InvalidTicket)?;

        parse_ticket(plaintext, unix_seconds(now)?)
    }
}

impl StreamTicketKeys {
    pub fn new(keys: Vec<StreamTicketKey>) -> Result<Self, StreamTicketError> {
        if keys.is_empty() || keys.len() > MAX_KEYS {
            return Err(StreamTicketError::InvalidKeyRing);
        }
        Ok(Self(keys))
    }

    pub fn verify(
        &self,
        encoded: &str,
        track_urn: &str,
    ) -> Result<StreamTicket, StreamTicketError> {
        let mut result = Err(StreamTicketError::InvalidTicket);
        for key in &self.0 {
            match key.verify(encoded, track_urn) {
                Ok(ticket) => return Ok(ticket),
                Err(StreamTicketError::Expired) => return Err(StreamTicketError::Expired),
                Err(StreamTicketError::InvalidClock) => {
                    return Err(StreamTicketError::InvalidClock);
                }
                Err(error) => result = Err(error),
            }
        }
        result
    }
}

impl fmt::Debug for StreamTicketKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StreamTicketKey([redacted])")
    }
}

impl fmt::Debug for StreamTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StreamTicket")
            .field("session_id", &self.session_id)
            .field(
                "secret_token",
                &self.secret_token.as_ref().map(|_| "[redacted]"),
            )
            .field("high_quality", &self.high_quality)
            .finish()
    }
}

fn aad(track_urn: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(AAD_PREFIX.len() + track_urn.len());
    aad.extend_from_slice(AAD_PREFIX);
    aad.extend_from_slice(track_urn.as_bytes());
    aad
}

fn parse_ticket(plaintext: &[u8], now: u64) -> Result<StreamTicket, StreamTicketError> {
    if plaintext.len() < HEADER_LEN || plaintext[0] != VERSION {
        return Err(StreamTicketError::InvalidTicket);
    }
    let flags = plaintext[1];
    if flags & !HIGH_QUALITY_FLAG != 0 {
        return Err(StreamTicketError::InvalidTicket);
    }

    let expires_at = u64::from_be_bytes(
        plaintext[2..10]
            .try_into()
            .map_err(|_| StreamTicketError::InvalidTicket)?,
    );
    if expires_at <= now || expires_at.saturating_sub(now) > MAX_LIFETIME.as_secs() {
        return Err(StreamTicketError::Expired);
    }

    let session_id = Uuid::from_bytes(
        plaintext[10..26]
            .try_into()
            .map_err(|_| StreamTicketError::InvalidTicket)?,
    );
    let secret_len = u16::from_be_bytes(
        plaintext[26..28]
            .try_into()
            .map_err(|_| StreamTicketError::InvalidTicket)?,
    ) as usize;
    let secret = plaintext
        .get(HEADER_LEN..)
        .filter(|secret| secret.len() == secret_len && secret.len() <= MAX_SECRET_LEN)
        .ok_or(StreamTicketError::InvalidTicket)?;
    let secret_token = if secret.is_empty() {
        None
    } else {
        Some(
            std::str::from_utf8(secret)
                .map_err(|_| StreamTicketError::InvalidTicket)?
                .to_owned(),
        )
    };

    Ok(StreamTicket {
        session_id,
        secret_token,
        high_quality: flags & HIGH_QUALITY_FLAG != 0,
    })
}

fn unix_seconds(time: SystemTime) -> Result<u64, StreamTicketError> {
    time.duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| StreamTicketError::InvalidClock)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "ZGV2LXN0cmVhbS10aWNrZXQta2V5LTAwMDAwMDAwMDA=";

    fn key() -> StreamTicketKey {
        StreamTicketKey::from_base64(KEY).unwrap()
    }

    fn other_key() -> StreamTicketKey {
        StreamTicketKey::from_base64("MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=").unwrap()
    }

    #[test]
    fn round_trips_session_and_private_secret() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000);
        let session_id = Uuid::from_u128(42);
        let encoded = key()
            .issue_at(
                session_id,
                "soundcloud:tracks:7",
                Some("private-secret"),
                true,
                Duration::from_secs(120),
                now,
            )
            .unwrap();

        let ticket = key()
            .verify_at(
                &encoded,
                "soundcloud:tracks:7",
                now + Duration::from_secs(30),
            )
            .unwrap();

        assert_eq!(ticket.session_id, session_id);
        assert_eq!(ticket.secret_token.as_deref(), Some("private-secret"));
        assert!(ticket.high_quality);
        assert!(!encoded.contains("private-secret"));
        assert!(!encoded.contains(&session_id.to_string()));
        assert!(!format!("{ticket:?}").contains("private-secret"));
    }

    #[test]
    fn rejects_a_ticket_for_another_track() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000);
        let encoded = key()
            .issue_at(
                Uuid::from_u128(42),
                "soundcloud:tracks:7",
                None,
                false,
                Duration::from_secs(120),
                now,
            )
            .unwrap();

        let error = key()
            .verify_at(&encoded, "soundcloud:tracks:8", now)
            .unwrap_err();

        assert_eq!(error, StreamTicketError::InvalidTicket);
    }

    #[test]
    fn rejects_expired_and_tampered_tickets() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000);
        let encoded = key()
            .issue_at(
                Uuid::from_u128(42),
                "soundcloud:tracks:7",
                None,
                false,
                Duration::from_secs(30),
                now,
            )
            .unwrap();

        assert_eq!(
            key().verify_at(
                &encoded,
                "soundcloud:tracks:7",
                now + Duration::from_secs(30)
            ),
            Err(StreamTicketError::Expired)
        );

        let mut tampered = encoded.into_bytes();
        let last = tampered.last_mut().unwrap();
        *last = if *last == b'A' { b'B' } else { b'A' };
        assert_eq!(
            key().verify_at(
                std::str::from_utf8(&tampered).unwrap(),
                "soundcloud:tracks:7",
                now
            ),
            Err(StreamTicketError::InvalidTicket)
        );
    }

    #[test]
    fn caps_ticket_lifetime() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000);
        let encoded = key()
            .issue_at(
                Uuid::from_u128(42),
                "soundcloud:tracks:7",
                None,
                false,
                Duration::from_secs(3_600),
                now,
            )
            .unwrap();

        assert!(
            key()
                .verify_at(
                    &encoded,
                    "soundcloud:tracks:7",
                    now + Duration::from_secs(299)
                )
                .is_ok()
        );
        assert_eq!(
            key().verify_at(
                &encoded,
                "soundcloud:tracks:7",
                now + Duration::from_secs(300)
            ),
            Err(StreamTicketError::Expired)
        );
    }

    #[test]
    fn key_ring_accepts_the_previous_key() {
        let encoded = key()
            .issue(
                Uuid::from_u128(42),
                "soundcloud:tracks:7",
                None,
                false,
                Duration::from_secs(120),
            )
            .unwrap();
        let keys = StreamTicketKeys::new(vec![other_key(), key()]).unwrap();

        assert!(keys.verify(&encoded, "soundcloud:tracks:7").is_ok());
    }
}
