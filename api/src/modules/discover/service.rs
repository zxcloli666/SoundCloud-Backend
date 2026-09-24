mod cache_runtime;
mod queries;
#[cfg(test)]
mod tag_list_tests;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use self::cache_runtime::LOCAL_CACHE_TTL;
use crate::cache::{CacheService, LocalTtlCache, SingleFlight};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedSummary {
    pub artists_count: i64,
    pub albums_count: i64,
    pub fresh_count: i64,
    pub fresh_window_days: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedTag {
    pub id: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedTagList {
    pub items: Vec<CachedTag>,
}

pub struct DiscoverService {
    pg: PgPool,
    cache: Arc<CacheService>,
    summary_flight: SingleFlight,
    tags_flight: SingleFlight,
    summary_cache: LocalTtlCache<CachedSummary>,
    tags_cache: LocalTtlCache<CachedTagList>,
}

impl DiscoverService {
    pub fn new(pg: PgPool, cache: Arc<CacheService>) -> Arc<Self> {
        Arc::new(Self {
            pg,
            cache,
            summary_flight: SingleFlight::new(),
            tags_flight: SingleFlight::new(),
            summary_cache: LocalTtlCache::new(LOCAL_CACHE_TTL),
            tags_cache: LocalTtlCache::new(LOCAL_CACHE_TTL),
        })
    }
}
