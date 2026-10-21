use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use chrono::Utc;
use mini_moka::sync::Cache;
use uuid::Uuid;

use crate::modules::chat::model::MessageRow;

pub const HOT_CACHE_BYTES: u64 = 192 * 1024 * 1024;
pub const TAIL_ROWS: usize = 256;
pub const TAIL_BYTES: usize = 512 * 1024;

const ROW_OVERHEAD: usize = 160;
const IDLE: Duration = Duration::from_secs(30 * 60);
const STRIPES: usize = 64;

#[derive(Default)]
struct Tail {
    rows: Vec<Arc<MessageRow>>,
    bytes: usize,
}

pub struct HotCache {
    tails: Cache<Uuid, Arc<Tail>>,
    stripes: Vec<Mutex<()>>,
}

fn weight(row: &MessageRow) -> usize {
    ROW_OVERHEAD + row.sender_user.len() + row.body.as_ref().map_or(0, Vec::len)
}

impl HotCache {
    pub fn new(max_bytes: u64) -> Self {
        Self {
            tails: Cache::builder()
                .max_capacity(max_bytes)
                .weigher(|_, tail: &Arc<Tail>| u32::try_from(tail.bytes).unwrap_or(u32::MAX))
                .time_to_idle(IDLE)
                .build(),
            stripes: (0..STRIPES).map(|_| Mutex::new(())).collect(),
        }
    }

    pub fn after(&self, conversation: Uuid, seq: i64) -> Option<Vec<MessageRow>> {
        let tail = self.tails.get(&conversation)?;
        let first = tail.rows.first()?.seq;
        let next = seq.checked_add(1)?;
        if next < first {
            return None;
        }
        let now = Utc::now();
        let mut rows = Vec::new();
        let newer = tail.rows.iter().filter(|row| row.seq > seq);
        for (expected, row) in (next..).zip(newer) {
            if row.seq != expected {
                return None;
            }
            if row.expires_at.is_none_or(|at| at > now) {
                rows.push(MessageRow::clone(row));
            }
        }
        Some(rows)
    }

    pub fn put(&self, row: MessageRow) {
        let stripe = &self.stripes[stripe_of(row.conversation_id)];
        let _held = stripe.lock().unwrap_or_else(PoisonError::into_inner);
        let conversation = row.conversation_id;
        let mut rows = self
            .tails
            .get(&conversation)
            .map(|tail| tail.rows.clone())
            .unwrap_or_default();
        match rows.binary_search_by_key(&row.seq, |cached| cached.seq) {
            Ok(at) => rows[at] = Arc::new(row),
            Err(0) if !rows.is_empty() => return,
            Err(at) => rows.insert(at, Arc::new(row)),
        }
        let mut bytes: usize = rows.iter().map(|cached| weight(cached)).sum();
        let mut drop = 0;
        while rows.len() - drop > TAIL_ROWS || (bytes > TAIL_BYTES && rows.len() - drop > 1) {
            bytes -= weight(&rows[drop]);
            drop += 1;
        }
        rows.drain(..drop);
        self.tails
            .insert(conversation, Arc::new(Tail { rows, bytes }));
    }

    pub fn forget(&self, conversation: Uuid) {
        self.tails.invalidate(&conversation);
    }
}

fn stripe_of(conversation: Uuid) -> usize {
    let bytes = conversation.as_bytes();
    usize::from(bytes[15]) % STRIPES
}

#[cfg(test)]
#[path = "tests/hot_cache_tests.rs"]
mod tests;
