use anyhow::Context;
use chat_wire::prost::Message;
use chat_wire::proto::{Inner, InnerResponse, Tier};
use chat_wire::seal::{RequestSealer, ResponseKey};
use chat_wire::sign::sign_request;

use crate::layout::Identity;

#[derive(Debug, Clone, Copy)]
pub struct Ock {
    pub version: u8,
    pub public: [u8; 32],
}

impl Ock {
    pub fn pinned(version: u8) -> anyhow::Result<Self> {
        let public = chat_wire::ock::pinned(version)
            .with_context(|| format!("ock-{version} is not pinned in chat-wire"))?;
        Ok(Self {
            version,
            public: *public,
        })
    }

    pub fn from_hex(version: u8, hex_key: &str) -> anyhow::Result<Self> {
        let bytes = hex::decode(hex_key.trim()).context("the ock public key is not hex")?;
        let public = bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("the ock public key is not 32 bytes"))?;
        Ok(Self { version, public })
    }
}

pub struct Call<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: &'a str,
    pub body: Vec<u8>,
}

pub struct Sealed {
    pub envelope: Vec<u8>,
    enc: [u8; 32],
    key: ResponseKey,
}

pub fn seal(
    identity: &Identity,
    ock: &Ock,
    call: Call<'_>,
    tier: Tier,
    max_bytes: u32,
    ts_ms: u64,
) -> anyhow::Result<Sealed> {
    let mut inner = Inner {
        ts_ms,
        session_id: Some(identity.session.to_string()),
        device_id: Some(identity.device.to_string()),
        method: call.method.to_owned(),
        path: call.path.to_owned(),
        query: call.query.to_owned(),
        body: call.body,
        tier: tier as i32,
        max_bytes,
        ..Inner::default()
    };
    let ikm: [u8; 32] = chat_wire::random_bytes().context("no randomness")?;
    let sealer = RequestSealer::new(ock.version, &ock.public, ikm).context("bad ock key")?;
    sign_request(&identity.auth_seed, &mut inner, sealer.enc());
    let sealed = sealer
        .seal(&inner.encode_to_vec())
        .context("the request does not seal")?;
    Ok(Sealed {
        envelope: sealed.envelope,
        enc: sealed.enc,
        key: sealed.response_key,
    })
}

impl Sealed {
    pub fn open(&self, body: &[u8]) -> anyhow::Result<InnerResponse> {
        let plain = self
            .key
            .open(&self.enc, body)
            .context("the answer does not open")?;
        InnerResponse::decode(plain.as_slice()).context("the answer is not an InnerResponse")
    }
}

#[cfg(test)]
#[path = "tests/wire_tests.rs"]
mod tests;
