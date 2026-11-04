use crate::modules::chat::{ChatResult, ChatState};

pub const EVERY_SECS: u64 = 15;

pub async fn run_once(chat: &ChatState) -> ChatResult<u64> {
    let mut conn = chat.redis.get().await?;
    let memory: String = redis::cmd("INFO")
        .arg("memory")
        .query_async(&mut conn)
        .await?;
    let errors: String = redis::cmd("INFO")
        .arg("errorstats")
        .query_async(&mut conn)
        .await?;
    publish(&memory, &errors);
    Ok(0)
}

pub fn publish(memory: &str, errors: &str) {
    if let Some(used) = field(memory, "used_memory") {
        metrics::gauge!("chat_redis_rt_used_bytes").set(used as f64);
    }
    if let Some(max) = field(memory, "maxmemory") {
        metrics::gauge!("chat_redis_rt_max_bytes").set(max as f64);
    }
    metrics::counter!("chat_redis_rt_rejected_total").absolute(rejected_writes(errors));
}

pub fn field(info: &str, name: &str) -> Option<u64> {
    info.lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))?
        .trim()
        .parse()
        .ok()
}

pub fn rejected_writes(errorstats: &str) -> u64 {
    errorstats
        .lines()
        .find_map(|line| line.strip_prefix("errorstat_OOM:"))
        .and_then(|stat| stat.split(',').find_map(|part| part.strip_prefix("count=")))
        .and_then(|count| count.trim().parse().ok())
        .unwrap_or(0)
}
