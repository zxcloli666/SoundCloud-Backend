use backend_contracts::remote::{BUFFER_SECS, RemoteCommand};
use deadpool_redis::Pool;
use redis::AsyncCommands;

use crate::error::AppResult;

const MICROS: i64 = 1_000_000;

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

    pub async fn since(&self, user: &str, cursor: i64) -> AppResult<Vec<RemoteCommand>> {
        let mut conn = self.redis.get().await?;
        let rows: Vec<String> = conn
            .zrangebyscore(self.key(user), format!("({cursor}"), "+inf")
            .await?;
        Ok(rows
            .iter()
            .filter_map(|row| serde_json::from_str(row).ok())
            .collect())
    }
}
