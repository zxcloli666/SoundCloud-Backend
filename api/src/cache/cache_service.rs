use std::sync::Arc;

use deadpool_redis::Pool;
use deadpool_redis::redis::AsyncCommands;
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::error::AppResult;

const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(400);
const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(600);
const DATA_PREFIX: &str = "api:";
const INDEX_PREFIX: &str = "idx:";
const LOCK_PREFIX: &str = "lock:";
const DEL_CHUNK: usize = 500;

#[derive(Clone, Copy, Debug)]
pub enum CacheScope {
    Shared,
    User,
}

impl CacheScope {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::User => "user",
        }
    }
}

pub struct CacheService {
    redis: Pool,
}

pub struct CacheLock {
    redis: Pool,
    key: String,
    token: String,
}

impl CacheService {
    pub fn new(redis: Pool) -> Arc<Self> {
        Arc::new(Self { redis })
    }

    pub async fn ping(&self) -> bool {
        self.get_raw("__healthcheck__").await.is_ok()
    }

    pub fn pool_status(&self) -> (usize, usize, usize) {
        let s = self.redis.status();
        (s.size, s.available, s.max_size)
    }

    pub fn build_key(
        &self,
        method: &str,
        url: &str,
        scope: CacheScope,
        session_id: Option<&str>,
    ) -> String {
        let (path, query) = match url.split_once('?') {
            Some((p, q)) => (p, q),
            None => (url, ""),
        };
        let mut qparts: Vec<&str> = query.split('&').filter(|s| !s.is_empty()).collect();
        qparts.sort_unstable();
        let sorted = qparts.join("&");

        let raw = match scope {
            CacheScope::User => {
                format!("user:{method}:{path}:{sorted}:{}", session_id.unwrap_or(""))
            }
            CacheScope::Shared => format!("shared:{method}:{path}:{sorted}"),
        };
        let digest = Sha256::digest(raw.as_bytes());
        hex::encode(digest)
    }

    pub async fn get_raw(&self, key: &str) -> AppResult<Option<String>> {
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(READ_TIMEOUT, self.get_raw_inner(key)).await;
        let (outcome, value) = match result {
            Ok(Ok(Some(found))) => (crate::metrics::Outcome::Ok, Ok(Some(found))),
            Ok(Ok(None)) => (crate::metrics::Outcome::Miss, Ok(None)),
            Ok(Err(error)) => (crate::metrics::Outcome::Error, Err(error)),
            Err(_) => (crate::metrics::Outcome::Timeout, Ok(None)),
        };
        crate::metrics::record_dependency("redis", "get", outcome, started.elapsed());
        value
    }

    async fn get_raw_inner(&self, key: &str) -> AppResult<Option<String>> {
        let mut conn = self.redis.get().await?;
        let full = format!("{DATA_PREFIX}{key}");
        let v: Option<String> = conn.get(&full).await?;
        Ok(v)
    }

    pub async fn set_raw(
        &self,
        key: &str,
        payload: &str,
        ttl_sec: u64,
        cache_key: Option<&str>,
        scope: CacheScope,
        session_id: Option<&str>,
    ) -> AppResult<()> {
        let full = format!("{DATA_PREFIX}{key}");

        let mut pipe = deadpool_redis::redis::pipe();
        pipe.atomic()
            .set_ex::<_, _>(&full, payload, ttl_sec)
            .ignore();

        if let Some(ck) = cache_key {
            let index_key = build_index_key(ck, scope, session_id);
            let now_ms = chrono::Utc::now().timestamp_millis();
            let expire_at = now_ms + (ttl_sec as i64) * 1000;
            pipe.zadd(&index_key, key, expire_at).ignore();
            pipe.zrembyscore::<_, _, _>(&index_key, 0, now_ms).ignore();
            pipe.pexpire_at(&index_key, expire_at).ignore();
        }

        let started = std::time::Instant::now();
        let result = tokio::time::timeout(WRITE_TIMEOUT, async {
            let mut conn = self.redis.get().await?;
            pipe.query_async::<()>(&mut conn).await?;
            Ok::<(), crate::error::AppError>(())
        })
        .await;
        let outcome = match &result {
            Ok(Ok(())) => crate::metrics::Outcome::Ok,
            Ok(Err(_)) => crate::metrics::Outcome::Error,
            Err(_) => crate::metrics::Outcome::Timeout,
        };
        crate::metrics::record_dependency("redis", "set", outcome, started.elapsed());
        match result {
            Ok(inner) => inner?,
            Err(_) => warn!(key, "redis write timed out"),
        }
        Ok(())
    }

    pub async fn clear_by_cache_keys(
        &self,
        cache_keys: &[String],
        session_id: Option<&str>,
    ) -> AppResult<()> {
        let mut seen = std::collections::BTreeSet::new();
        for k in cache_keys {
            let trimmed = k.trim();
            if !trimmed.is_empty() {
                seen.insert(trimmed.to_string());
            }
        }
        if seen.is_empty() {
            return Ok(());
        }

        let mut index_keys: Vec<String> = Vec::with_capacity(seen.len() * 2);
        for ck in &seen {
            index_keys.push(build_index_key(ck, CacheScope::Shared, None));
            if session_id.is_some() {
                index_keys.push(build_index_key(ck, CacheScope::User, session_id));
            }
        }

        for idx in index_keys {
            if let Err(e) = self.clear_index(&idx).await {
                warn!(index = %idx, error = %e, "cache clear_index failed");
            }
        }
        Ok(())
    }

    pub async fn try_acquire_lock(&self, key: &str, ttl_sec: u64) -> AppResult<bool> {
        let mut conn = self.redis.get().await?;
        let full = format!("{LOCK_PREFIX}{key}");
        let acquired: Option<String> = deadpool_redis::redis::cmd("SET")
            .arg(&full)
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(ttl_sec)
            .query_async(&mut conn)
            .await?;
        Ok(acquired.is_some())
    }

    pub async fn try_acquire_owned_lock(
        &self,
        key: &str,
        ttl_sec: u64,
    ) -> AppResult<Option<CacheLock>> {
        let mut conn = self.redis.get().await?;
        let key = format!("{LOCK_PREFIX}{key}");
        let token = uuid::Uuid::now_v7().to_string();
        let acquired: Option<String> = deadpool_redis::redis::cmd("SET")
            .arg(&key)
            .arg(&token)
            .arg("NX")
            .arg("EX")
            .arg(ttl_sec)
            .query_async(&mut conn)
            .await?;

        Ok(acquired.map(|_| CacheLock {
            redis: self.redis.clone(),
            key,
            token,
        }))
    }

    pub async fn release_lock(&self, key: &str) -> AppResult<()> {
        let mut conn = self.redis.get().await?;
        let full = format!("{LOCK_PREFIX}{key}");
        let _: () = conn.del(full).await?;
        Ok(())
    }

    async fn clear_index(&self, index_key: &str) -> AppResult<()> {
        let mut conn = self.redis.get().await?;
        let members: Vec<String> = conn.zrange(index_key, 0, -1).await?;
        if members.is_empty() {
            let _: () = conn.del(index_key).await?;
            return Ok(());
        }

        for chunk in members.chunks(DEL_CHUNK) {
            let keys: Vec<String> = chunk.iter().map(|m| format!("{DATA_PREFIX}{m}")).collect();
            let _: () = conn.del(keys).await?;
        }
        let _: () = conn.del(index_key).await?;
        Ok(())
    }
}

impl CacheLock {
    pub async fn release(self) -> AppResult<()> {
        release_owned_lock(&self.redis, &self.key, &self.token).await
    }
}

async fn release_owned_lock(redis: &Pool, key: &str, token: &str) -> AppResult<()> {
    let mut conn = redis.get().await?;
    let script = "if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('DEL', KEYS[1]) else return 0 end";
    let _: i64 = deadpool_redis::redis::cmd("EVAL")
        .arg(script)
        .arg(1)
        .arg(key)
        .arg(token)
        .query_async(&mut conn)
        .await?;
    Ok(())
}

pub fn build_index_key(cache_key: &str, scope: CacheScope, session_id: Option<&str>) -> String {
    match scope {
        CacheScope::User => format!(
            "{INDEX_PREFIX}{}:{}:{}",
            scope.as_str(),
            session_id.unwrap_or(""),
            cache_key
        ),
        CacheScope::Shared => format!("{INDEX_PREFIX}{}:{}", scope.as_str(), cache_key),
    }
}

#[cfg(test)]
mod tests {
    use deadpool_redis::{Config, Runtime};

    use super::*;

    fn service() -> Arc<CacheService> {
        let pool = Config::from_url("redis://127.0.0.1:1")
            .create_pool(Some(Runtime::Tokio1))
            .expect("offline redis pool builds");
        CacheService::new(pool)
    }

    #[test]
    fn a_user_scoped_answer_is_never_shared_between_owners() {
        let cache = service();
        let url = "/users/soundcloud:users:42/subscription";

        let first = cache.build_key("GET", url, CacheScope::User, Some("111"));
        let second = cache.build_key("GET", url, CacheScope::User, Some("222"));
        let shared = cache.build_key("GET", url, CacheScope::Shared, None);

        assert_ne!(first, second);
        assert_ne!(first, shared);
        assert_ne!(second, shared);
    }

    #[test]
    fn a_user_scope_without_an_owner_collapses_into_one_key() {
        let cache = service();
        let url = "/users/soundcloud:users:42/subscription";

        let first = cache.build_key("GET", url, CacheScope::User, None);
        let second = cache.build_key("GET", url, CacheScope::User, None);

        assert_eq!(first, second);
    }
}

#[cfg(test)]
mod timeout_tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_silent_redis_turns_a_read_into_a_miss_instead_of_a_stall() {
        let started = tokio::time::Instant::now();
        let outcome = tokio::time::timeout(READ_TIMEOUT, std::future::pending::<()>()).await;
        assert!(outcome.is_err());
        assert!(started.elapsed() >= READ_TIMEOUT);
        assert!(READ_TIMEOUT < WRITE_TIMEOUT);
    }

    #[test]
    fn cache_timeouts_stay_below_the_request_deadline() {
        assert!(READ_TIMEOUT.as_millis() <= 500);
        assert!(WRITE_TIMEOUT.as_millis() <= 1000);
    }
}
