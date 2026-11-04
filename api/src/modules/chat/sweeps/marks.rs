use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;

use mini_moka::sync::Cache;
use uuid::Uuid;

use crate::modules::chat::{ChatResult, ChatState, rt_keys};

pub const EVERY_SECS: u64 = 30;
pub const SCAN_COUNT: usize = 1_000;
pub const KEYS_PER_RUN: usize = 50_000;
pub const SCANS_PER_RUN: usize = 200;
pub const MARKS_PER_WRITE: usize = 1_000;
pub const REMEMBERED: u64 = 200_000;
pub const REMEMBER_FOR: Duration = Duration::from_secs(6 * 3_600);

pub struct Flusher {
    cursor: u64,
    flushed: Cache<Uuid, u64>,
}

struct Pending {
    conversation: Uuid,
    digest: u64,
    marks: HashMap<String, i64>,
}

impl Flusher {
    pub fn new() -> Self {
        Self {
            cursor: 0,
            flushed: Cache::builder()
                .max_capacity(REMEMBERED)
                .time_to_live(REMEMBER_FOR)
                .build(),
        }
    }

    pub async fn run_once(&mut self, chat: &ChatState) -> ChatResult<u64> {
        let prefix = read_marks_prefix();
        let pattern = format!("{prefix}*");
        let mut scanned = 0;
        let mut scans = 0;
        let mut written = 0;
        loop {
            let (next, keys) = scan(chat, self.cursor, &pattern).await?;
            scans += 1;
            scanned += keys.len();
            let conversations: Vec<Uuid> = keys
                .iter()
                .filter_map(|key| conversation_of(&prefix, key))
                .collect();
            written += self.flush(chat, &conversations).await?;
            self.cursor = next;
            if next == 0
                || scanned >= KEYS_PER_RUN
                || scans >= SCANS_PER_RUN
                || chat.shutdown.is_cancelled()
            {
                break;
            }
        }
        Ok(written)
    }

    async fn flush(&self, chat: &ChatState, conversations: &[Uuid]) -> ChatResult<u64> {
        if conversations.is_empty() {
            return Ok(0);
        }
        let mut pipe = redis::pipe();
        for conversation in conversations {
            pipe.hgetall(rt_keys::read_marks(*conversation));
        }
        let replies: Vec<HashMap<String, String>> = {
            let mut conn = chat.redis.get().await?;
            pipe.query_async(&mut conn).await?
        };
        let pending: Vec<Pending> = conversations
            .iter()
            .zip(replies)
            .map(|(conversation, raw)| (conversation, seqs(raw)))
            .map(|(conversation, marks)| Pending {
                conversation: *conversation,
                digest: digest(&marks),
                marks,
            })
            .filter(|pending| {
                !pending.marks.is_empty()
                    && self.flushed.get(&pending.conversation) != Some(pending.digest)
            })
            .collect();
        let mut written = 0;
        let rows: Vec<(Uuid, &str, i64)> = pending
            .iter()
            .flat_map(|pending| {
                pending
                    .marks
                    .iter()
                    .map(|(user, seq)| (pending.conversation, user.as_str(), *seq))
            })
            .collect();
        for chunk in rows.chunks(MARKS_PER_WRITE) {
            written += write(chat, chunk).await?;
        }
        for pending in pending {
            self.flushed.insert(pending.conversation, pending.digest);
        }
        Ok(written)
    }
}

impl Default for Flusher {
    fn default() -> Self {
        Self::new()
    }
}

pub fn read_marks_prefix() -> String {
    let probe = rt_keys::read_marks(Uuid::nil());
    let nil = Uuid::nil().to_string();
    probe.strip_suffix(&nil).unwrap_or(&probe).to_owned()
}

pub fn conversation_of(prefix: &str, key: &str) -> Option<Uuid> {
    key.strip_prefix(prefix)?.parse().ok()
}

pub fn seqs(raw: HashMap<String, String>) -> HashMap<String, i64> {
    raw.into_iter()
        .filter_map(|(user, seq)| Some((user, seq.parse().ok()?)))
        .collect()
}

pub fn digest(marks: &HashMap<String, i64>) -> u64 {
    let mut sorted: Vec<(&String, &i64)> = marks.iter().collect();
    sorted.sort();
    let mut hasher = DefaultHasher::new();
    sorted.hash(&mut hasher);
    hasher.finish()
}

async fn scan(chat: &ChatState, cursor: u64, pattern: &str) -> ChatResult<(u64, Vec<String>)> {
    let mut conn = chat.redis.get().await?;
    let page: (u64, Vec<String>) = redis::cmd("SCAN")
        .arg(cursor)
        .arg("MATCH")
        .arg(pattern)
        .arg("COUNT")
        .arg(SCAN_COUNT)
        .query_async(&mut conn)
        .await?;
    Ok(page)
}

async fn write(chat: &ChatState, rows: &[(Uuid, &str, i64)]) -> ChatResult<u64> {
    let conversations: Vec<Uuid> = rows.iter().map(|row| row.0).collect();
    let users: Vec<String> = rows.iter().map(|row| row.1.to_owned()).collect();
    let seqs: Vec<i64> = rows.iter().map(|row| row.2).collect();
    let done = sqlx::query_file!(
        "queries/chat/sweeps/flush_read_marks.sql",
        &conversations,
        &users,
        &seqs
    )
    .execute(&chat.pg)
    .await?
    .rows_affected();
    Ok(done)
}
