use hpke::aead::ChaCha20Poly1305;
use hpke::kdf::HkdfSha256;
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable, Kem, OpModeR, OpModeS, Serializable};

use crate::WireError;
use crate::aead::NONCE_LEN;
use crate::random::Seeded;

const WRAP_LABEL: &[u8] = b"scd-chat/ck/v1";
const SEAL_LABEL: &[u8] = b"scd-chat/ck-seal/v1";
const ENC_LEN: usize = 32;

pub fn wrap_aad(conv: &[u8; 16], epoch: u32, kek_v: u16) -> Vec<u8> {
    let mut aad = WRAP_LABEL.to_vec();
    aad.extend_from_slice(conv);
    aad.extend_from_slice(&epoch.to_be_bytes());
    aad.extend_from_slice(&kek_v.to_be_bytes());
    aad
}

pub fn seal_info(conv: &[u8; 16], epoch: u32, device_id: &[u8; 16]) -> Vec<u8> {
    let mut info = SEAL_LABEL.to_vec();
    info.extend_from_slice(conv);
    info.extend_from_slice(&epoch.to_be_bytes());
    info.extend_from_slice(device_id);
    info
}

pub fn wrap(
    kek: &[u8; 32],
    kek_v: u16,
    conv: &[u8; 16],
    epoch: u32,
    ck: &[u8; 32],
    nonce: [u8; NONCE_LEN],
) -> Result<Vec<u8>, WireError> {
    crate::aead::seal(kek, nonce, ck, &wrap_aad(conv, epoch, kek_v))
}

pub fn unwrap(
    kek: &[u8; 32],
    kek_v: u16,
    conv: &[u8; 16],
    epoch: u32,
    wrapped: &[u8],
) -> Result<[u8; 32], WireError> {
    let ck = crate::aead::open(kek, wrapped, &wrap_aad(conv, epoch, kek_v))?;
    ck.try_into().map_err(|_| WireError::Open)
}

pub fn box_public(box_secret: &[u8; 32]) -> Result<[u8; 32], WireError> {
    let secret = <X25519HkdfSha256 as Kem>::PrivateKey::from_bytes(box_secret)
        .map_err(|_| WireError::Key)?;
    Ok(X25519HkdfSha256::sk_to_pk(&secret).to_bytes().into())
}

pub fn seal_for_device(
    box_pub: &[u8],
    conv: &[u8; 16],
    epoch: u32,
    device_id: &[u8; 16],
    ck: &[u8; 32],
    ikm_e: [u8; 32],
) -> Result<Vec<u8>, WireError> {
    let recipient =
        <X25519HkdfSha256 as Kem>::PublicKey::from_bytes(box_pub).map_err(|_| WireError::Key)?;
    let (encapped, mut context) =
        hpke::setup_sender::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256, _>(
            &OpModeS::Base,
            &recipient,
            &seal_info(conv, epoch, device_id),
            &mut Seeded::new(ikm_e),
        )
        .map_err(|_| WireError::Seal)?;
    let ciphertext = context.seal(ck, &[]).map_err(|_| WireError::Seal)?;
    let mut sealed = encapped.to_bytes().to_vec();
    sealed.extend(ciphertext);
    Ok(sealed)
}

pub fn open_for_device(
    box_secret: &[u8; 32],
    conv: &[u8; 16],
    epoch: u32,
    device_id: &[u8; 16],
    sealed: &[u8],
) -> Result<[u8; 32], WireError> {
    if sealed.len() <= ENC_LEN {
        return Err(WireError::Open);
    }
    let (enc, ciphertext) = sealed.split_at(ENC_LEN);
    let secret = <X25519HkdfSha256 as Kem>::PrivateKey::from_bytes(box_secret)
        .map_err(|_| WireError::Key)?;
    let encapped =
        <X25519HkdfSha256 as Kem>::EncappedKey::from_bytes(enc).map_err(|_| WireError::Open)?;
    let mut context = hpke::setup_receiver::<ChaCha20Poly1305, HkdfSha256, X25519HkdfSha256>(
        &OpModeR::Base,
        &secret,
        &encapped,
        &seal_info(conv, epoch, device_id),
    )
    .map_err(|_| WireError::Open)?;
    let ck = context.open(ciphertext, &[]).map_err(|_| WireError::Open)?;
    ck.try_into().map_err(|_| WireError::Open)
}
