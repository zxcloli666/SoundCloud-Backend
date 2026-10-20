use prost::Message;

use crate::WireError;
use crate::aead::NONCE_LEN;
use crate::proto::Envelope;

pub const ENVELOPE_VERSION: u32 = 1;
pub const TEXT_MAX: usize = 4096;
pub const PAD_BLOCK: usize = 256;
pub const EMBEDS_MAX: usize = 3;

const LABEL: &[u8] = b"scd-chat/msg/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Post,
    Edit,
    React,
    Pin,
    Meta,
    System,
}

impl Class {
    pub fn as_str(self) -> &'static str {
        match self {
            Class::Post => "post",
            Class::Edit => "edit",
            Class::React => "react",
            Class::Pin => "pin",
            Class::Meta => "meta",
            Class::System => "system",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "post" => Some(Class::Post),
            "edit" => Some(Class::Edit),
            "react" => Some(Class::React),
            "pin" => Some(Class::Pin),
            "meta" => Some(Class::Meta),
            "system" => Some(Class::System),
            _ => None,
        }
    }
}

pub struct MessageAad {
    pub conv: [u8; 16],
    pub epoch: u32,
    pub sender_device: [u8; 16],
    pub client_id: [u8; 16],
    pub class: Class,
}

impl MessageAad {
    pub fn bytes(&self) -> Vec<u8> {
        let mut aad = LABEL.to_vec();
        aad.extend_from_slice(&self.conv);
        aad.extend_from_slice(&self.epoch.to_be_bytes());
        aad.extend_from_slice(&self.sender_device);
        aad.extend_from_slice(&self.client_id);
        aad.extend_from_slice(self.class.as_str().as_bytes());
        aad
    }
}

pub fn encode_padded(envelope: &Envelope) -> Vec<u8> {
    let mut padded = envelope.clone();
    padded.padding.clear();
    let padding = padding_len(padded.encoded_len());
    padded.padding = vec![0; padding];
    padded.encode_to_vec()
}

fn padding_len(unpadded: usize) -> usize {
    let mut target = (unpadded + 3).div_ceil(PAD_BLOCK) * PAD_BLOCK;
    loop {
        for prefix in [2, 3] {
            let padding = target - unpadded - prefix;
            if 1 + prost::encoding::encoded_len_varint(padding as u64) == prefix {
                return padding;
            }
        }
        target += PAD_BLOCK;
    }
}

pub fn seal(
    ck: &[u8; 32],
    aad: &MessageAad,
    nonce: [u8; NONCE_LEN],
    envelope: &Envelope,
) -> Result<Vec<u8>, WireError> {
    crate::aead::seal(ck, nonce, &encode_padded(envelope), &aad.bytes())
}

pub fn open(ck: &[u8; 32], aad: &MessageAad, body: &[u8]) -> Result<Envelope, WireError> {
    let plaintext = crate::aead::open(ck, body, &aad.bytes())?;
    Envelope::decode(plaintext.as_slice()).map_err(|_| WireError::Decode)
}
