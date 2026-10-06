use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use sc_transport::SearchType;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::cache::cache_service::CacheScope;
use crate::cache::{CacheService, KeyedCoalesce, ListPageResult};
use crate::common::admission::{AdmissionRejection, Endpoint, PublicAdmission};
use crate::error::{AppError, AppResult};
use crate::sc::ScReadService;

const MAX_PAGE: i64 = 24;
const MAX_LIMIT: i64 = 50;
const DEPTH: i64 = 300;
const MAX_CHUNKS: usize = 15;
const MAX_QUERY_CHARS: usize = 200;
const CHUNK_TTL_SECONDS: u64 = 300;
const FAILURE_TTL_SECONDS: u64 = 10;
const DEFAULT_RETRY_SECONDS: i64 = 10;
const BUDGET: Duration = Duration::from_secs(8);

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Chunk {
    items: Vec<Value>,
    next_href: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Failure {
    retry_after: i64,
}

type Fetched = Result<Chunk, i64>;

pub struct SoundCloudSearch {
    reads: Arc<ScReadService>,
    cache: Arc<CacheService>,
    admission: Arc<PublicAdmission>,
    flights: KeyedCoalesce<Fetched>,
}

impl SoundCloudSearch {
    pub fn new(
        reads: Arc<ScReadService>,
        cache: Arc<CacheService>,
        admission: Arc<PublicAdmission>,
    ) -> Arc<Self> {
        Arc::new(Self {
            reads,
            cache,
            admission,
            flights: KeyedCoalesce::new(),
        })
    }

    pub async fn page(
        &self,
        session_id: Uuid,
        ty: SearchType,
        q: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<ListPageResult<Value>> {
        let page = page.clamp(0, MAX_PAGE);
        let limit = limit.clamp(1, MAX_LIMIT);
        let q: String = q.trim().chars().take(MAX_QUERY_CHARS).collect();
        let start = page * limit;
        let end = ((page + 1) * limit).min(DEPTH);
        if q.is_empty() || start >= DEPTH {
            return Ok(empty_page(page, limit));
        }
        let prefix = chunk_prefix(ty, &q);
        let walk = self.walk(session_id, ty, &q, &prefix, end);
        let (items, upstream_has_more) = tokio::time::timeout(BUDGET, walk)
            .await
            .map_err(|_| unavailable(DEFAULT_RETRY_SECONDS))??;
        let has_more = (items.len() as i64 > end || upstream_has_more) && end < DEPTH;
        Ok(ListPageResult {
            collection: items
                .into_iter()
                .skip(start as usize)
                .take((end - start) as usize)
                .collect(),
            page,
            page_size: limit,
            has_more,
        })
    }

    async fn walk(
        &self,
        session_id: Uuid,
        ty: SearchType,
        q: &str,
        prefix: &str,
        end: i64,
    ) -> AppResult<(Vec<Value>, bool)> {
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        for index in 0..MAX_CHUNKS {
            let chunk = self
                .chunk(session_id, ty, q, &format!("{prefix}:{index}"), cursor)
                .await?;
            items.extend(chunk.items);
            cursor = chunk.next_href;
            if cursor.is_none() {
                return Ok((items, false));
            }
            if items.len() as i64 >= end {
                break;
            }
        }
        Ok((items, cursor.is_some()))
    }

    async fn chunk(
        &self,
        session_id: Uuid,
        ty: SearchType,
        q: &str,
        key: &str,
        cursor: Option<String>,
    ) -> AppResult<Chunk> {
        if let Some(chunk) = self.read::<Chunk>(key).await {
            return Ok(chunk);
        }
        let failure_key = format!("{key}:fail");
        if let Some(failure) = self.read::<Failure>(&failure_key).await {
            return Err(unavailable(failure.retry_after));
        }
        self.admission
            .check_session(Endpoint::SoundCloudSearch, session_id)
            .await
            .map_err(busy)?;
        let fetched = self
            .flights
            .run(key, || async {
                Ok::<Fetched, AppError>(self.fetch(ty, q, key, &failure_key, cursor).await)
            })
            .await?;
        fetched.map_err(unavailable)
    }

    async fn fetch(
        &self,
        ty: SearchType,
        q: &str,
        key: &str,
        failure_key: &str,
        cursor: Option<String>,
    ) -> Fetched {
        let chunk = match self.reads.search(ty, q, cursor.as_deref()).await {
            Ok(page) => Chunk {
                items: page.items,
                next_href: page.next_href,
            },
            Err(AppError::ScApi {
                status: 400 | 404 | 422,
                ..
            }) => Chunk::default(),
            Err(error) => {
                tracing::warn!(%error, search = ty.as_str(), "SoundCloud search failed");
                let retry_after = self.retry_after().await;
                self.write(failure_key, &Failure { retry_after }, FAILURE_TTL_SECONDS)
                    .await;
                return Err(retry_after);
            }
        };
        self.write(key, &chunk, CHUNK_TTL_SECONDS).await;
        Ok(chunk)
    }

    async fn retry_after(&self) -> i64 {
        self.reads
            .search_cooldown()
            .await
            .map_or(DEFAULT_RETRY_SECONDS, |left| {
                (left.as_secs() as i64).clamp(5, 60)
            })
    }

    async fn read<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        let raw = self.cache.get_raw(key).await.ok()??;
        serde_json::from_str(&raw).ok()
    }

    async fn write<T: Serialize>(&self, key: &str, value: &T, ttl: u64) {
        let Ok(json) = serde_json::to_string(value) else {
            return;
        };
        let _ = self
            .cache
            .set_raw(key, &json, ttl, None, CacheScope::Shared, None)
            .await;
    }
}

pub(super) fn chunk_prefix(ty: SearchType, q: &str) -> String {
    let folded = q
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "sc-search:v1:{}:{}",
        ty.as_str(),
        hex::encode(Sha256::digest(folded.as_bytes()))
    )
}

fn empty_page(page: i64, limit: i64) -> ListPageResult<Value> {
    ListPageResult {
        collection: Vec::new(),
        page,
        page_size: limit,
        has_more: false,
    }
}

pub(crate) fn unavailable(retry_after: i64) -> AppError {
    AppError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        "soundcloud_search_unavailable",
        "SoundCloud search is unavailable right now",
    )
    .with_retry_after(retry_after)
}

pub(crate) fn busy(rejection: AdmissionRejection) -> AppError {
    let retry_after = match rejection {
        AdmissionRejection::Limited {
            retry_after_seconds,
        } => i64::try_from(retry_after_seconds).unwrap_or(60),
        AdmissionRejection::Unavailable => 1,
    };
    AppError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        "soundcloud_search_busy",
        "SoundCloud search is busy, try again shortly",
    )
    .with_retry_after(retry_after)
}
