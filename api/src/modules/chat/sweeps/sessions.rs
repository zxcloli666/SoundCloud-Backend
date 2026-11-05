use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::modules::chat::caller::Cold;
use crate::modules::chat::errors::BusySource;
use crate::modules::chat::{ChatError, ChatResult, ChatState};

pub const INTERVAL: Duration = Duration::from_secs(10);
pub const JITTER: f64 = 0.2;
pub const PER_CALL: usize = 1_000;

const CHECKED: &str = "chat_session_check_hashes";
const CALL_SECONDS: &str = "chat_session_check_call_seconds";
const EVICTED_TOTAL: &str = "chat_session_check_evicted_total";
const SKIPPED_TOTAL: &str = "chat_session_check_skipped_total";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pass {
    pub calls: Vec<usize>,
    pub evicted: usize,
    pub skipped: bool,
}

pub async fn run(chat: Arc<ChatState>) {
    loop {
        tokio::select! {
            () = chat.shutdown.cancelled() => return,
            () = tokio::time::sleep(jittered(INTERVAL)) => {}
        }
        check(&chat).await;
    }
}

pub fn jittered(interval: Duration) -> Duration {
    interval.mul_f64(1.0 + JITTER * (2.0 * rand::random::<f64>() - 1.0))
}

pub async fn check(chat: &ChatState) -> Pass {
    let used = chat.callers.take_used();
    metrics::histogram!(CHECKED).record(used.len() as f64);
    let mut pass = Pass::default();
    for (index, chunk) in used.chunks(PER_CALL).enumerate() {
        match alive(chat, chunk).await {
            Ok(alive) => {
                pass.calls.push(chunk.len());
                pass.evicted += evict_dead(chat, chunk, &alive);
            }
            Err(error) => {
                let why = if error.is_busy() { "busy" } else { "failed" };
                metrics::counter!(SKIPPED_TOTAL, "why" => why).increment(1);
                tracing::warn!(
                    code = error.code(),
                    "chat sessions wait for the next liveness pass"
                );
                chat.callers
                    .mark_used(used[index * PER_CALL..].iter().cloned());
                pass.skipped = true;
                return pass;
            }
        }
    }
    pass
}

async fn alive(chat: &ChatState, chunk: &[Vec<u8>]) -> ChatResult<HashSet<Vec<u8>>> {
    if saturated(chat) {
        return Err(ChatError::busy(BusySource::ColdSession));
    }
    chat.callers
        .cold(Cold::Alive, Duration::ZERO, async {
            let started = Instant::now();
            let alive = sqlx::query_file_scalar!("queries/chat/sessions/alive.sql", chunk)
                .fetch_all(&chat.pg)
                .await;
            metrics::histogram!(CALL_SECONDS).record(started.elapsed().as_secs_f64());
            Ok(alive?.into_iter().collect())
        })
        .await
}

fn saturated(chat: &ChatState) -> bool {
    let max = chat.pg.options().get_max_connections();
    chat.pg.num_idle() == 0 && chat.pg.size() >= max
}

fn evict_dead(chat: &ChatState, chunk: &[Vec<u8>], alive: &HashSet<Vec<u8>>) -> usize {
    let mut evicted = 0;
    for hash in chunk.iter().filter(|hash| !alive.contains(*hash)) {
        let Some(entry) = chat.callers.evict_hash(hash) else {
            continue;
        };
        if let Some(bind) = entry.bind {
            chat.devices.forget(&entry.user_id, &[bind.device_id]);
        }
        evicted += 1;
    }
    metrics::counter!(EVICTED_TOTAL).increment(evicted as u64);
    evicted
}
