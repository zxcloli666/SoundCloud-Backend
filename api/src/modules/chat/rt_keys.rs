use std::time::Duration;

use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const VERSION_TTL: Duration = Duration::from_secs(7 * 86_400);
pub const DELTAS_KEPT: isize = 256;
pub const TYPING_TTL: Duration = Duration::from_secs(6);
pub const MARKS_TTL: Duration = Duration::from_secs(7 * 86_400);
pub const SEEN_TTL: Duration = Duration::from_secs(600);
pub const REPLAY_TTL: Duration = Duration::from_secs(300);
pub const LEAD_TTL: Duration = Duration::from_secs(30);
pub const LOADTEST_SESSION_TTL: Duration = Duration::from_secs(2 * 3_600);

pub const ROOMS_ALIVE: &str = "rooms:alive";
pub const SEEN: &str = "chat:seen";
pub const LEAD: &str = "chat:lead";

pub fn version(user_id: &str) -> String {
    format!("chat:v:{user_id}")
}

pub fn deltas(user_id: &str) -> String {
    format!("chat:d:{user_id}")
}

pub fn conversation_seq(conversation: Uuid) -> String {
    format!("chat:c:{conversation}")
}

pub fn typing(conversation: Uuid, user_id: &str) -> String {
    format!("chat:t:{conversation}:{user_id}")
}

pub fn read_marks(conversation: Uuid) -> String {
    format!("chat:r:{conversation}")
}

pub fn delivered_marks(conversation: Uuid) -> String {
    format!("chat:dl:{conversation}")
}

pub fn slow_mode(conversation: Uuid, user_id: &str) -> String {
    format!("chat:slow:{conversation}:{user_id}")
}

pub fn replay(enc: &[u8]) -> String {
    let digest = Sha256::digest(enc);
    format!("chat:rp:{}", hex::encode(&digest[..16]))
}

pub fn limit(bucket: &str, user_id: &str) -> String {
    format!("chat:lim:{bucket}:{user_id}")
}

pub fn loadtest_session(session: Uuid) -> String {
    format!("chat:lt:sess:{session}")
}

#[cfg(test)]
#[path = "tests/rt_keys_tests.rs"]
mod tests;
