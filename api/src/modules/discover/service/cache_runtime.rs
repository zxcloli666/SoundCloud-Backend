use std::future::Future;
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tracing::warn;

use crate::cache::LocalTtlCache;
use crate::cache::cache_service::{CacheLock, CacheScope};
use crate::error::{AppError, AppResult};

use super::{CachedSummary, CachedTagList, DiscoverService};

const COMPUTE_TIMEOUT: Duration = Duration::from_secs(25);
const LOCK_TTL_SECONDS: u64 = 30;
const WAIT_TIMEOUT: Duration = Duration::from_secs(2);
const WAIT_STEP: Duration = Duration::from_millis(50);
const REDIS_TIMEOUT: Duration = Duration::from_millis(500);
const REDIS_TTL_SECONDS: u64 = 3 * 60 * 60;

pub(super) const LOCAL_CACHE_TTL: Duration = Duration::from_secs(30 * 60);

const SUMMARY_KEY: &str = "discover:summary:v1";
const TAGS_KEY: &str = "discover:tags:v1";
const SUMMARY_LOCK_KEY: &str = "discover:summary:v1:build";
const TAGS_LOCK_KEY: &str = "discover:tags:v1:build";

impl DiscoverService {
    pub async fn cached_summary(&self) -> AppResult<CachedSummary> {
        self.summary_flight
            .get_or_load(
                || self.read_cache(SUMMARY_KEY, &self.summary_cache),
                || self.load_summary(),
            )
            .await
    }

    pub async fn cached_tag_list(&self) -> AppResult<CachedTagList> {
        self.tags_flight
            .get_or_load(
                || self.read_cache(TAGS_KEY, &self.tags_cache),
                || self.load_tag_list(),
            )
            .await
    }

    async fn load_summary(&self) -> AppResult<CachedSummary> {
        loop {
            match self.acquire_cache_lock(SUMMARY_LOCK_KEY).await {
                Ok(Some(lock)) => {
                    let result = match self
                        .read_remote_cache(SUMMARY_KEY, &self.summary_cache)
                        .await
                    {
                        Ok(Some(cached)) => Ok(cached),
                        Ok(None) => self.compute_and_cache_summary().await,
                        Err(error) => {
                            warn!(error = %error, "discover summary cache recheck failed");
                            self.compute_and_cache_summary().await
                        }
                    };
                    self.release_cache_lock(lock, "summary").await;
                    return result;
                }
                Ok(None) => match self.wait_for_cache(SUMMARY_KEY, &self.summary_cache).await {
                    Ok(Some(cached)) => return Ok(cached),
                    Ok(None) => {}
                    Err(error) => {
                        warn!(error = %error, "discover summary cache wait failed");
                        return self.compute_and_cache_summary().await;
                    }
                },
                Err(error) => {
                    warn!(error = %error, "discover summary lock unavailable");
                    return self.compute_and_cache_summary().await;
                }
            }
        }
    }

    async fn load_tag_list(&self) -> AppResult<CachedTagList> {
        loop {
            match self.acquire_cache_lock(TAGS_LOCK_KEY).await {
                Ok(Some(lock)) => {
                    let result = match self.read_remote_cache(TAGS_KEY, &self.tags_cache).await {
                        Ok(Some(cached)) => Ok(cached),
                        Ok(None) => self.compute_and_cache_tag_list().await,
                        Err(error) => {
                            warn!(error = %error, "discover tags cache recheck failed");
                            self.compute_and_cache_tag_list().await
                        }
                    };
                    self.release_cache_lock(lock, "tags").await;
                    return result;
                }
                Ok(None) => match self.wait_for_cache(TAGS_KEY, &self.tags_cache).await {
                    Ok(Some(cached)) => return Ok(cached),
                    Ok(None) => {}
                    Err(error) => {
                        warn!(error = %error, "discover tags cache wait failed");
                        return self.compute_and_cache_tag_list().await;
                    }
                },
                Err(error) => {
                    warn!(error = %error, "discover tags lock unavailable");
                    return self.compute_and_cache_tag_list().await;
                }
            }
        }
    }

    async fn compute_and_cache_summary(&self) -> AppResult<CachedSummary> {
        let summary = with_timeout(
            COMPUTE_TIMEOUT,
            "discover summary query timed out",
            self.compute_summary(),
        )
        .await?;
        self.summary_cache.set(summary.clone());
        self.write_cache(SUMMARY_KEY, &summary).await;
        Ok(summary)
    }

    async fn compute_and_cache_tag_list(&self) -> AppResult<CachedTagList> {
        let tags = with_timeout(
            COMPUTE_TIMEOUT,
            "discover tags query timed out",
            self.compute_tag_list(),
        )
        .await?;
        self.tags_cache.set(tags.clone());
        self.write_cache(TAGS_KEY, &tags).await;
        Ok(tags)
    }

    async fn acquire_cache_lock(&self, key: &str) -> AppResult<Option<CacheLock>> {
        with_timeout(
            REDIS_TIMEOUT,
            "discover cache lock timed out",
            self.cache.try_acquire_owned_lock(key, LOCK_TTL_SECONDS),
        )
        .await
    }

    async fn release_cache_lock(&self, lock: CacheLock, cache: &'static str) {
        if let Err(error) = with_timeout(
            REDIS_TIMEOUT,
            "discover cache lock release timed out",
            lock.release(),
        )
        .await
        {
            warn!(cache, error = %error, "discover cache lock release failed");
        }
    }

    async fn read_cache<T>(&self, key: &str, local: &LocalTtlCache<T>) -> Option<T>
    where
        T: Clone + DeserializeOwned,
    {
        if let Some(cached) = local.get() {
            return Some(cached);
        }
        match self.read_remote_cache(key, local).await {
            Ok(cached) => cached,
            Err(error) => {
                warn!(key, error = %error, "discover redis cache read failed");
                None
            }
        }
    }

    async fn read_remote_cache<T>(
        &self,
        key: &str,
        local: &LocalTtlCache<T>,
    ) -> AppResult<Option<T>>
    where
        T: Clone + DeserializeOwned,
    {
        let raw = with_timeout(
            REDIS_TIMEOUT,
            "discover redis cache read timed out",
            self.cache.get_raw(key),
        )
        .await?;
        let Some(raw) = raw else {
            return Ok(None);
        };
        let cached: T = match serde_json::from_str(&raw) {
            Ok(cached) => cached,
            Err(error) => {
                warn!(key, error = %error, "discover redis cache payload invalid");
                return Ok(None);
            }
        };
        local.set(cached.clone());
        Ok(Some(cached))
    }

    async fn wait_for_cache<T>(&self, key: &str, local: &LocalTtlCache<T>) -> AppResult<Option<T>>
    where
        T: Clone + DeserializeOwned,
    {
        let deadline = tokio::time::Instant::now() + WAIT_TIMEOUT;
        loop {
            if let Some(cached) = local.get() {
                return Ok(Some(cached));
            }
            tokio::time::sleep(WAIT_STEP).await;
            if let Some(cached) = self.read_remote_cache(key, local).await? {
                return Ok(Some(cached));
            }
            if tokio::time::Instant::now() >= deadline {
                return Ok(None);
            }
        }
    }

    async fn write_cache<T: Serialize>(&self, key: &str, value: &T) {
        let payload = match serde_json::to_string(value) {
            Ok(payload) => payload,
            Err(error) => {
                warn!(key, error = %error, "discover cache serialization failed");
                return;
            }
        };
        if let Err(error) = with_timeout(
            REDIS_TIMEOUT,
            "discover redis cache write timed out",
            self.cache.set_raw(
                key,
                &payload,
                REDIS_TTL_SECONDS,
                None,
                CacheScope::Shared,
                None,
            ),
        )
        .await
        {
            warn!(key, error = %error, "discover redis cache write failed");
        }
    }
}

pub(super) async fn with_timeout<T>(
    timeout: Duration,
    message: &'static str,
    future: impl Future<Output = AppResult<T>>,
) -> AppResult<T> {
    tokio::time::timeout(timeout, future)
        .await
        .map_err(|_| AppError::internal(message))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn with_timeout_stops_pending_operation() {
        let result = with_timeout(
            Duration::from_millis(1),
            "timed out",
            std::future::pending::<AppResult<()>>(),
        )
        .await;

        assert!(matches!(result, Err(AppError::Internal(message)) if message == "timed out"));
    }
}
