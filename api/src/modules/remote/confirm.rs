use deadpool_redis::Pool;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

pub const MAX_MISSES: i64 = 5;
const LOCK_SECS: u64 = 70;
const DAY_SECS: i64 = 24 * 3600;
const MISS_TTL_SECS: i64 = 700;

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
pub struct PageTicket {
    pub status: Status,
    pub client_id: String,
    pub redirect_uri: String,
    pub state: String,
    pub scope: String,
    pub location: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SalutTicket {
    pub status: Status,
    pub user: String,
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

    pub async fn claim(&self, code_id: uuid::Uuid) -> AppResult<bool> {
        let mut conn = self.redis.get().await.map_err(redis_error)?;
        let set: Option<String> = redis::cmd("SET")
            .arg(self.key(&format!("lock:{code_id}")))
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(LOCK_SECS)
            .query_async(&mut conn)
            .await
            .map_err(redis_error)?;
        Ok(set.is_some())
    }

    pub async fn release(&self, code_id: uuid::Uuid) {
        if let Ok(mut conn) = self.redis.get().await {
            let _ = redis::cmd("DEL")
                .arg(self.key(&format!("lock:{code_id}")))
                .query_async::<()>(&mut conn)
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

    pub async fn asked_today(&self, user: &str) -> AppResult<i64> {
        let day = chrono::Utc::now().timestamp() / DAY_SECS;
        self.count(&format!("day:{user}:{day}"), DAY_SECS).await
    }

    pub async fn missed(&self, code_id: uuid::Uuid) -> AppResult<i64> {
        self.count(&format!("misses:{code_id}"), MISS_TTL_SECS)
            .await
    }
}
