use anyhow::Context;
use chat_wire::msg::{self, Class, ENVELOPE_VERSION, MessageAad};
use chat_wire::proto::envelope::Body;
use chat_wire::proto::{Envelope, Text};
use rand::Rng;
use uuid::Uuid;

const MARK: &str = "lt";
const FILLER: &str = "the quick brown fox jumps over the lazy dog while the bass drops ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub run: u32,
    pub sender: usize,
    pub sent_ms: u64,
}

pub struct Sealing<'a> {
    pub ck: &'a [u8; 32],
    pub conversation: Uuid,
    pub epoch: i32,
    pub sender_device: Uuid,
    pub client_id: Uuid,
}

impl Sealing<'_> {
    fn aad(&self) -> MessageAad {
        MessageAad {
            conv: *self.conversation.as_bytes(),
            epoch: self.epoch.max(0) as u32,
            sender_device: *self.sender_device.as_bytes(),
            client_id: *self.client_id.as_bytes(),
            class: Class::Post,
        }
    }
}

pub fn text(stamp: Stamp, rng: &mut impl Rng) -> String {
    let filler_len = rng.gen_range(8..=FILLER.len());
    format!(
        "{MARK}|{}|{}|{}|{}",
        stamp.run,
        stamp.sender,
        stamp.sent_ms,
        &FILLER[..filler_len]
    )
}

pub fn read_stamp(text: &str) -> Option<Stamp> {
    let mut parts = text.splitn(5, '|');
    if parts.next()? != MARK {
        return None;
    }
    Some(Stamp {
        run: parts.next()?.parse().ok()?,
        sender: parts.next()?.parse().ok()?,
        sent_ms: parts.next()?.parse().ok()?,
    })
}

pub fn seal(sealing: &Sealing<'_>, text: String) -> anyhow::Result<Vec<u8>> {
    let envelope = Envelope {
        v: ENVELOPE_VERSION,
        body: Some(Body::Text(Text {
            text,
            spans: Vec::new(),
        })),
        ..Envelope::default()
    };
    let nonce = chat_wire::random_bytes().context("no randomness")?;
    msg::seal(sealing.ck, &sealing.aad(), nonce, &envelope).context("the message does not seal")
}

pub fn open(sealing: &Sealing<'_>, body: &[u8]) -> anyhow::Result<String> {
    let envelope =
        msg::open(sealing.ck, &sealing.aad(), body).context("the message does not open")?;
    match envelope.body {
        Some(Body::Text(text)) => Ok(text.text),
        _ => anyhow::bail!("the message is not text"),
    }
}

#[cfg(test)]
#[path = "tests/content_tests.rs"]
mod tests;
