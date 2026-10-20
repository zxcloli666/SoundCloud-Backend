use hpke::aead::{AeadCtxS, ChaCha20Poly1305};
use hpke::kdf::HkdfSha256;
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable, Kem, OpModeR, OpModeS, Serializable};

use crate::WireError;
use crate::ock::OckSecret;
use crate::random::Seeded;

pub use crate::aead::NONCE_LEN;

pub const VERSION: u8 = 1;
pub const MAX_ENVELOPE: usize = 64 * 1024;
pub const ENC_LEN: usize = 32;

const HEADER_LEN: usize = 2 + ENC_LEN;
const REQUEST_INFO: &[u8] = b"scd-chat/x/v1";
const RESPONSE_LABEL: &[u8] = b"scd-chat/resp/v1";

type Context = AeadCtxS<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>;

pub struct RequestSealer {
    ock_v: u8,
    enc: [u8; ENC_LEN],
    context: Context,
}

pub struct SealedRequest {
    pub envelope: Vec<u8>,
    pub enc: [u8; ENC_LEN],
    pub response_key: ResponseKey,
}

pub struct OpenedRequest {
    pub ock_v: u8,
    pub enc: [u8; ENC_LEN],
    pub inner: Vec<u8>,
    pub response_key: ResponseKey,
}

impl RequestSealer {
    pub fn new(ock_v: u8, ock_pub: &[u8; 32], ikm_e: [u8; 32]) -> Result<Self, WireError> {
        let recipient = <X25519HkdfSha256 as Kem>::PublicKey::from_bytes(ock_pub)
            .map_err(|_| WireError::Key)?;
        let (encapped, context) =
            hpke::setup_sender::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256, _>(
                &OpModeS::Base,
                &recipient,
                REQUEST_INFO,
                &mut Seeded::new(ikm_e),
            )
            .map_err(|_| WireError::Seal)?;
        Ok(Self {
            ock_v,
            enc: encapped.to_bytes().into(),
            context,
        })
    }

    pub fn enc(&self) -> &[u8; ENC_LEN] {
        &self.enc
    }

    pub fn seal(mut self, inner: &[u8]) -> Result<SealedRequest, WireError> {
        let header = [VERSION, self.ock_v];
        let ciphertext = self
            .context
            .seal(inner, &header)
            .map_err(|_| WireError::Seal)?;
        let mut envelope = Vec::with_capacity(HEADER_LEN + ciphertext.len());
        envelope.extend_from_slice(&header);
        envelope.extend_from_slice(&self.enc);
        envelope.extend(ciphertext);
        if envelope.len() > MAX_ENVELOPE {
            return Err(WireError::TooLarge);
        }
        let mut response_key = [0u8; 32];
        self.context
            .export(RESPONSE_LABEL, &mut response_key)
            .map_err(|_| WireError::Seal)?;
        Ok(SealedRequest {
            envelope,
            enc: self.enc,
            response_key: ResponseKey(response_key),
        })
    }
}

pub fn ock_version(envelope: &[u8]) -> Result<u8, WireError> {
    if envelope.len() > MAX_ENVELOPE {
        return Err(WireError::TooLarge);
    }
    if envelope.len() <= HEADER_LEN {
        return Err(WireError::Envelope);
    }
    if envelope[0] != VERSION {
        return Err(WireError::Version(envelope[0]));
    }
    Ok(envelope[1])
}

pub fn open_request(envelope: &[u8], secret: &OckSecret) -> Result<OpenedRequest, WireError> {
    let ock_v = ock_version(envelope)?;
    let (header, rest) = envelope.split_at(2);
    let (enc, ciphertext) = rest.split_at(ENC_LEN);
    let encapped =
        <X25519HkdfSha256 as Kem>::EncappedKey::from_bytes(enc).map_err(|_| WireError::Envelope)?;
    let mut context = hpke::setup_receiver::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
        &OpModeR::Base,
        secret.key(),
        &encapped,
        REQUEST_INFO,
    )
    .map_err(|_| WireError::Open)?;
    let inner = context
        .open(ciphertext, header)
        .map_err(|_| WireError::Open)?;
    let mut response_key = [0u8; 32];
    context
        .export(RESPONSE_LABEL, &mut response_key)
        .map_err(|_| WireError::Open)?;
    Ok(OpenedRequest {
        ock_v,
        enc: enc.try_into().map_err(|_| WireError::Envelope)?,
        inner,
        response_key: ResponseKey(response_key),
    })
}

pub struct ResponseKey([u8; 32]);

impl ResponseKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn seal(
        &self,
        enc: &[u8; ENC_LEN],
        nonce: [u8; NONCE_LEN],
        answer: &[u8],
    ) -> Result<Vec<u8>, WireError> {
        crate::aead::seal(&self.0, nonce, answer, enc)
    }

    pub fn open(&self, enc: &[u8; ENC_LEN], body: &[u8]) -> Result<Vec<u8>, WireError> {
        crate::aead::open(&self.0, body, enc)
    }
}
