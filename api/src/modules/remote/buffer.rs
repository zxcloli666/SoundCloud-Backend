use std::collections::HashSet;

use backend_contracts::remote::{BUFFER_SECS, RemoteCommand};
use deadpool_redis::Pool;
use redis::Script;
use uuid::Uuid;

use crate::error::AppResult;

const MICROS: i64 = 1_000_000;
const SEQ_TTL_SECS: i64 = 7 * 24 * 3600;

const NEXT_SEQ: &str = r#"
local current = tonumber(redis.call('GET', KEYS[1]) or '0')
local next = tonumber(ARGV[1])
if current + 1 > next then next = current + 1 end
redis.call('SET', KEYS[1], next, 'EX', ARGV[2])
return next
"#;

const HANDSHAKE_SEQ: &str = r#"
local current = redis.call('GET', KEYS[1])
if current then return tonumber(current) end
redis.call('SET', KEYS[1], ARGV[1], 'EX', ARGV[2])
return tonumber(ARGV[1])
"#;

#[derive(Clone)]
pub struct Buffer {
    redis: Pool,
    prefix: String,
}

impl Buffer {
    pub fn new(redis: Pool, prefix: impl Into<String>) -> Self {
        Self {
            redis,
            prefix: prefix.into(),
        }
    }

    fn key(&self, user: &str) -> String {
        format!("{}:buf:{user}", self.prefix)
    }

    fn seq_key(&self, user: &str) -> String {
        format!("{}:seq:{user}", self.prefix)
    }

    fn answered_key(&self, user: &str) -> String {
        format!("{}:answered:{user}", self.prefix)
    }

    pub async fn handshake_seq(&self, user: &str, floor: i64) -> AppResult<i64> {
        let mut conn = self.redis.get().await?;
        let seq: i64 = Script::new(HANDSHAKE_SEQ)
            .key(self.seq_key(user))
            .arg(floor)
            .arg(SEQ_TTL_SECS)
            .invoke_async(&mut conn)
            .await?;
        Ok(seq)
    }

    pub async fn next_seq(&self, user: &str, floor: i64) -> AppResult<i64> {
        let mut conn = self.redis.get().await?;
        let next: i64 = Script::new(NEXT_SEQ)
            .key(self.seq_key(user))
            .arg(floor)
            .arg(SEQ_TTL_SECS)
            .invoke_async(&mut conn)
            .await?;
        Ok(next)
    }

    pub async fn push(&self, command: &RemoteCommand) -> AppResult<()> {
        let key = self.key(&command.user);
        let body = serde_json::to_string(command).map_err(|error| {
            crate::error::AppError::internal(format!("command encode: {error}"))
        })?;
        let oldest = command.seq - BUFFER_SECS as i64 * MICROS;
        let mut conn = self.redis.get().await?;
        redis::pipe()
            .zadd(&key, body, command.seq)
            .ignore()
            .zrembyscore(&key, i64::MIN, oldest)
            .ignore()
            .expire(&key, BUFFER_SECS as i64 + 10)
            .ignore()
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn mark_answered(&self, user: &str, id: Uuid) -> AppResult<()> {
        let key = self.answered_key(user);
        let mut conn = self.redis.get().await?;
        redis::pipe()
            .sadd(&key, id.to_string())
            .ignore()
            .expire(&key, BUFFER_SECS as i64 + 10)
            .ignore()
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    pub async fn since(&self, user: &str, cursor: i64) -> AppResult<Vec<RemoteCommand>> {
        let mut conn = self.redis.get().await?;
        let (rows, answered): (Vec<String>, HashSet<String>) = redis::pipe()
            .zrangebyscore(self.key(user), format!("({cursor}"), "+inf")
            .smembers(self.answered_key(user))
            .query_async(&mut conn)
            .await?;
        Ok(rows
            .iter()
            .filter_map(|row| serde_json::from_str::<RemoteCommand>(row).ok())
            .filter(|command| !answered.contains(&command.id.to_string()))
            .collect())
    }
}
