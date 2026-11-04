use backend_contracts::remote::LinkProvider;
use deadpool_redis::Pool;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

pub const MAX_CODE_ASKS: i64 = 5;
const LOCK_SECS: u64 = 70;
const RELEASE_OWN_LOCK: &str = "if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('DEL', KEYS[1]) else return 0 end";
const DAY_SECS: i64 = 24 * 3600;
const CODE_ASKS_TTL_SECS: i64 = 700;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Approved,
    Denied,
    Offline,
    Silent,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Confirming {
    pub code_id: Uuid,
    pub holder: String,
    pub user: String,
    pub provider: LinkProvider,
    pub command: Uuid,
    pub deadline_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageTicket {
    pub status: Status,
    pub client_id: String,
    pub redirect_uri: String,
    pub state: String,
    pub scope: String,
    pub location: redact::Secret<String>,
    #[serde(default)]
    pub hint: String,
    #[serde(default)]
    pub confirming: Option<Confirming>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SalutTicket {
    pub status: Status,
    pub user: String,
    #[serde(default)]
    pub hint: String,
    #[serde(default)]
    pub confirming: Option<Confirming>,
}

pub struct LinkConfirm {
    redis: Pool,
    prefix: String,
}

fn redis_error(error: impl std::fmt::Display) -> AppError {
    AppError::internal(format!("link confirm store: {error}"))
}

impl LinkConfirm {
    pub fn new(redis: Pool, prefix: impl Into<String>) -> Self {
        Self {
            redis,
            prefix: prefix.into(),
        }
    }

    fn key(&self, part: &str) -> String {
        format!("{}:{part}", self.prefix)
    }

    pub async fn put<T: Serialize>(&self, part: &str, value: &T, ttl_secs: u64) -> AppResult<()> {
        let body = serde_json::to_string(value).map_err(redis_error)?;
        let mut conn = self.redis.get().await.map_err(redis_error)?;
        redis::cmd("SET")
            .arg(self.key(part))
            .arg(body)
            .arg("EX")
            .arg(ttl_secs)
            .query_async::<()>(&mut conn)
            .await
            .map_err(redis_error)
    }

    pub async fn get<T: DeserializeOwned>(&self, part: &str) -> AppResult<Option<T>> {
        let mut conn = self.redis.get().await.map_err(redis_error)?;
        let body: Option<String> = redis::cmd("GET")
            .arg(self.key(part))
            .query_async(&mut conn)
            .await
            .map_err(redis_error)?;
        Ok(body.and_then(|body| serde_json::from_str(&body).ok()))
    }

    pub async fn take<T: DeserializeOwned>(&self, part: &str) -> AppResult<Option<T>> {
        let mut conn = self.redis.get().await.map_err(redis_error)?;
        let body: Option<String> = redis::cmd("GETDEL")
            .arg(self.key(part))
            .query_async(&mut conn)
            .await
            .map_err(redis_error)?;
        Ok(body.and_then(|body| serde_json::from_str(&body).ok()))
    }

    fn lock_key(&self, code_id: Uuid) -> String {
        self.key(&format!("lock:{code_id}"))
    }

    pub async fn claim(&self, code_id: Uuid) -> AppResult<Option<String>> {
        let holder = Uuid::now_v7().simple().to_string();
        let mut conn = self.redis.get().await.map_err(redis_error)?;
        let set: Option<String> = redis::cmd("SET")
            .arg(self.lock_key(code_id))
            .arg(&holder)
            .arg("NX")
            .arg("EX")
            .arg(LOCK_SECS)
            .query_async(&mut conn)
            .await
            .map_err(redis_error)?;
        Ok(set.map(|_| holder))
    }

    pub async fn release(&self, code_id: Uuid, holder: &str) {
        if let Ok(mut conn) = self.redis.get().await {
            let _ = redis::cmd("EVAL")
                .arg(RELEASE_OWN_LOCK)
                .arg(1)
                .arg(self.lock_key(code_id))
                .arg(holder)
                .query_async::<i64>(&mut conn)
                .await;
        }
    }

    async fn count(&self, part: &str, ttl_secs: i64) -> AppResult<i64> {
        let mut conn = self.redis.get().await.map_err(redis_error)?;
        let key = self.key(part);
        let (count, _): (i64, i64) = redis::pipe()
            .cmd("INCR")
            .arg(&key)
            .cmd("EXPIRE")
            .arg(&key)
            .arg(ttl_secs)
            .arg("NX")
            .query_async(&mut conn)
            .await
            .map_err(redis_error)?;
        Ok(count)
    }

    fn day_key(user: &str) -> String {
        let day = chrono::Utc::now().timestamp() / DAY_SECS;
        format!("day:{user}:{day}")
    }

    pub async fn asked_today(&self, user: &str) -> AppResult<i64> {
        self.count(&Self::day_key(user), DAY_SECS).await
    }

    pub async fn asks_so_far(&self, user: &str) -> AppResult<i64> {
        let mut conn = self.redis.get().await.map_err(redis_error)?;
        let count: Option<i64> = redis::cmd("GET")
            .arg(self.key(&Self::day_key(user)))
            .query_async(&mut conn)
            .await
            .map_err(redis_error)?;
        Ok(count.unwrap_or(0))
    }

    fn code_asks_key(code_id: Uuid) -> String {
        format!("asks:{code_id}")
    }

    pub async fn note_code_ask(&self, code_id: Uuid) -> AppResult<i64> {
        self.count(&Self::code_asks_key(code_id), CODE_ASKS_TTL_SECS)
            .await
    }

    pub async fn code_asks(&self, code_id: Uuid) -> AppResult<i64> {
        let mut conn = self.redis.get().await.map_err(redis_error)?;
        let count: Option<i64> = redis::cmd("GET")
            .arg(self.key(&Self::code_asks_key(code_id)))
            .query_async(&mut conn)
            .await
            .map_err(redis_error)?;
        Ok(count.unwrap_or(0))
    }

    #[cfg(test)]
    pub async fn ttl(&self, part: &str) -> AppResult<i64> {
        let mut conn = self.redis.get().await.map_err(redis_error)?;
        redis::cmd("TTL")
            .arg(self.key(part))
            .query_async(&mut conn)
            .await
            .map_err(redis_error)
    }
}

#[cfg(test)]
mod tests {
    use deadpool_redis::{Config, Runtime};

    use super::*;

    fn confirm() -> LinkConfirm {
        let url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned());
        let redis = Config::from_url(url)
            .create_pool(Some(Runtime::Tokio1))
            .expect("a redis pool");
        LinkConfirm::new(redis, format!("voice-test-{}", Uuid::now_v7()))
    }

    #[tokio::test]
    #[ignore = "requires a local Redis"]
    async fn a_stale_attempt_does_not_release_a_newer_lock() {
        let confirm = confirm();
        let code = Uuid::now_v7();
        let stale = confirm
            .claim(code)
            .await
            .expect("redis")
            .expect("the first claim");
        let mut conn = confirm.redis.get().await.expect("redis");
        redis::cmd("DEL")
            .arg(confirm.lock_key(code))
            .query_async::<()>(&mut conn)
            .await
            .expect("the lock runs out");
        let fresh = confirm
            .claim(code)
            .await
            .expect("redis")
            .expect("a claim after the lock ran out");

        confirm.release(code, &stale).await;
        assert!(
            confirm.claim(code).await.expect("redis").is_none(),
            "the newer lock is still held"
        );

        confirm.release(code, &fresh).await;
        assert!(confirm.claim(code).await.expect("redis").is_some());
    }
}
