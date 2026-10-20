use uuid::Uuid;

use crate::modules::chat::model::MessageRow;

pub const HOT_CACHE_BYTES: u64 = 192 * 1024 * 1024;

pub struct HotCache {
    max_bytes: u64,
}

impl HotCache {
    pub fn new(max_bytes: u64) -> Self {
        Self { max_bytes }
    }

    pub fn after(&self, _conversation: Uuid, _seq: i64) -> Option<Vec<MessageRow>> {
        None
    }

    pub fn put(&self, _row: MessageRow) {}

    pub fn forget(&self, _conversation: Uuid) {}
}
