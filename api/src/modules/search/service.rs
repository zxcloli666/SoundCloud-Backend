use std::sync::Arc;

use serde::Serialize;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::cache::cache_service::CacheScope;
use crate::cache::{CacheService, ListPageResult, build_list_cache_key};
use crate::error::AppResult;
use crate::modules::enrich::dto as enrich_dto;
use crate::modules::search::repository;
use catalog_normalize::normalize_name;
const TTL_SECONDS: u64 = 60;
pub const MIN_QUERY_LEN: usize = 2;
pub const MAX_QUERY_LEN: usize = 128;
pub const MAX_PAGE: i64 = 24;
pub const MAX_LIMIT: i64 = 50;

pub struct SearchService {
    pg: PgPool,
    cache: Arc<CacheService>,
    flights: crate::cache::KeyedCoalesce<String>,
}

impl SearchService {
    pub fn new(pg: PgPool, cache: Arc<CacheService>) -> Arc<Self> {
        Arc::new(Self {
            pg,
            cache,
            flights: crate::cache::KeyedCoalesce::new(),
        })
    }
    fn normalize_query(raw: &str) -> Option<String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        let truncated: String = trimmed.chars().take(MAX_QUERY_LEN).collect();
        let normalized = normalize_name(&truncated);
        if normalized.chars().count() < MIN_QUERY_LEN {
            return None;
        }
        Some(normalized)
    }

    fn clamp_page_limit(page: i64, limit: i64) -> (i64, i64) {
        let page = page.clamp(0, MAX_PAGE);
        let limit = limit.clamp(1, MAX_LIMIT);
        (page, limit)
    }
    async fn cached<F, Fut>(&self, cache_key: &str, compute: F) -> AppResult<Value>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = AppResult<Value>>,
    {
        if let Ok(Some(raw)) = self.cache.get_raw(cache_key).await
            && let Ok(v) = serde_json::from_str::<Value>(&raw)
        {
            return Ok(v);
        }
        let json = self
            .flights
            .run(cache_key, || async {
                let value = compute().await?;
                let json = serde_json::to_string(&value).unwrap_or_default();
                if !json.is_empty() {
                    let _ = self
                        .cache
                        .set_raw(
                            cache_key,
                            &json,
                            TTL_SECONDS,
                            None,
                            CacheScope::Shared,
                            None,
                        )
                        .await;
                }
                Ok::<String, crate::error::AppError>(json)
            })
            .await?;
        serde_json::from_str::<Value>(&json)
            .map_err(|error| crate::error::AppError::internal(error.to_string()))
    }

    pub async fn tracks(
        &self,
        query: &super::query::TrackSearchQuery,
        page: i64,
        limit: i64,
    ) -> AppResult<ListPageResult<Value>> {
        let (page, limit) = Self::clamp_page_limit(page, limit);
        super::query::validate_access(query.access.as_deref())?;
        let ids = super::query::parse_ids(query.ids.as_deref(), "tracks")?;
        let genres = super::query::parse_terms(query.genres.as_deref(), "genres")?;
        let tags = super::query::parse_terms(query.tags.as_deref(), "tags")?;
        let owner = super::query::parse_owner(query.user_urn.as_deref())?;
        let raw_query: String = query
            .q
            .as_deref()
            .unwrap_or_default()
            .trim()
            .chars()
            .take(MAX_QUERY_LEN)
            .collect();
        let normalized = Self::normalize_query(&raw_query);
        if normalized.is_none()
            && (!raw_query.trim().is_empty()
                || (ids.is_none() && genres.is_none() && tags.is_none() && owner.is_none()))
        {
            return Ok(empty_page(page, limit));
        }
        let filters = repository::TrackSearch {
            query: normalized.as_ref().map(|_| raw_query.as_str()),
            owner: owner.as_deref(),
            ids: ids.as_deref(),
            genres: genres.as_deref(),
            tags: tags.as_deref(),
        };
        let (mut collection, has_more) =
            repository::search_tracks(&self.pg, &filters, page, limit).await?;
        enrich_dto::apply_to_tracks(&self.pg, &mut collection).await?;
        Ok(ListPageResult {
            collection,
            page,
            page_size: limit,
            has_more: has_more && page < MAX_PAGE,
        })
    }

    pub async fn playlists(
        &self,
        query: &super::query::PlaylistSearchQuery,
        page: i64,
        limit: i64,
    ) -> AppResult<ListPageResult<Value>> {
        let (page, limit) = Self::clamp_page_limit(page, limit);
        super::query::validate_access(query.access.as_deref())?;
        if query
            .show_tracks
            .as_deref()
            .is_some_and(|value| !matches!(value, "false" | "0"))
        {
            return Err(crate::error::AppError::bad_request(
                "Search returns playlist metadata; use GET /playlists/{urn}/tracks for membership",
            ));
        }
        let owner = super::query::parse_owner(query.user_urn.as_deref())?;
        let raw_query: String = query
            .q
            .as_deref()
            .unwrap_or_default()
            .trim()
            .chars()
            .take(MAX_QUERY_LEN)
            .collect();
        let normalized = Self::normalize_query(&raw_query);
        if normalized.is_none() && (!raw_query.trim().is_empty() || owner.is_none()) {
            return Ok(empty_page(page, limit));
        }
        let (collection, has_more) = repository::search_playlists(
            &self.pg,
            normalized.as_ref().map(|_| raw_query.as_str()),
            owner.as_deref(),
            page,
            limit,
        )
        .await?;
        Ok(ListPageResult {
            collection,
            page,
            page_size: limit,
            has_more: has_more && page < MAX_PAGE,
        })
    }

    pub async fn users(
        &self,
        q: &str,
        ids: Option<&str>,
        page: i64,
        limit: i64,
    ) -> AppResult<ListPageResult<Value>> {
        let (page, limit) = Self::clamp_page_limit(page, limit);
        let ids = super::query::parse_ids(ids, "users")?;
        let raw_query: String = q.trim().chars().take(MAX_QUERY_LEN).collect();
        let query = Self::normalize_query(&raw_query);
        if (query.is_none() && !q.trim().is_empty()) || (query.is_none() && ids.is_none()) {
            return Ok(empty_page(page, limit));
        }
        let (collection, has_more) = repository::search_users(
            &self.pg,
            query.as_ref().map(|_| raw_query.as_str()),
            ids.as_deref(),
            page,
            limit,
        )
        .await?;
        Ok(ListPageResult {
            collection,
            page,
            page_size: limit,
            has_more: has_more && page < MAX_PAGE,
        })
    }

    pub async fn artists(
        &self,
        q: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<ListPageResult<Value>> {
        let Some(q_norm) = Self::normalize_query(q) else {
            return Ok(empty_page(page, limit));
        };
        let (page, limit) = Self::clamp_page_limit(page, limit);

        let params: Vec<(&str, String)> = vec![
            ("q", q_norm.clone()),
            ("page", page.to_string()),
            ("limit", limit.to_string()),
        ];
        let key = build_list_cache_key("search-db-artists", &params);

        let value = self
            .cached(&key, || async {
                let (rows, has_more) =
                    repository::search_artists(&self.pg, &q_norm, page, limit).await?;
                let items: Vec<Value> = rows.into_iter().map(artist_to_value).collect();
                Ok(serde_json::to_value(PageEnvelope {
                    collection: items,
                    page,
                    page_size: limit,
                    has_more: has_more && page < MAX_PAGE,
                })
                .unwrap_or(Value::Null))
            })
            .await?;
        Ok(decode_page(value, page, limit))
    }

    pub async fn albums(&self, q: &str, page: i64, limit: i64) -> AppResult<ListPageResult<Value>> {
        let Some(q_norm) = Self::normalize_query(q) else {
            return Ok(empty_page(page, limit));
        };
        let (page, limit) = Self::clamp_page_limit(page, limit);

        let params: Vec<(&str, String)> = vec![
            ("q", q_norm.clone()),
            ("page", page.to_string()),
            ("limit", limit.to_string()),
        ];
        let key = build_list_cache_key("search-db-albums", &params);

        let value = self
            .cached(&key, || async {
                let (rows, has_more) =
                    repository::search_albums(&self.pg, &q_norm, page, limit).await?;
                let items: Vec<Value> = rows.into_iter().map(album_to_value).collect();
                Ok(serde_json::to_value(PageEnvelope {
                    collection: items,
                    page,
                    page_size: limit,
                    has_more: has_more && page < MAX_PAGE,
                })
                .unwrap_or(Value::Null))
            })
            .await?;
        Ok(decode_page(value, page, limit))
    }
}

#[derive(Debug, Serialize)]
struct PageEnvelope {
    collection: Vec<Value>,
    page: i64,
    page_size: i64,
    has_more: bool,
}

fn decode_page(v: Value, fallback_page: i64, fallback_limit: i64) -> ListPageResult<Value> {
    let mut result = serde_json::from_value::<ListPageResult<Value>>(v)
        .unwrap_or_else(|_| empty_page(fallback_page, fallback_limit));
    result.has_more &= result.page < MAX_PAGE;
    result
}

fn empty_page(page: i64, limit: i64) -> ListPageResult<Value> {
    let (page, limit) = SearchService::clamp_page_limit(page, limit);
    ListPageResult {
        collection: Vec::new(),
        page,
        page_size: limit,
        has_more: false,
    }
}

fn artist_to_value(r: repository::ArtistSearchRow) -> Value {
    json!({
        "id": r.id,
        "name": r.name,
        "country": r.country,
        "avatar_url": r.avatar_url,
        "confidence": r.confidence,
        "track_count_primary": r.track_count_primary,
        "track_count_featured": r.track_count_featured,
        "album_count": r.album_count_denorm,
        "monthly_listeners": r.monthly_listeners,
        "trending": r.trending_score,
        "tags": crate::modules::discover::tags::canonicalize_tags(r.tags),
        "star": r.is_star,
        "aura_id": if r.is_star { r.star_aura_id } else { None },
        "custom_hex": if r.is_star { r.star_custom_hex } else { None },
    })
}

fn album_to_value(r: repository::AlbumSearchRow) -> Value {
    let release_month = r
        .release_date
        .map(|d| d.format("%-m").to_string().parse::<i32>().unwrap_or(0));
    json!({
        "id": r.id,
        "title": r.title,
        "type": r.kind,
        "release_year": r.release_year,
        "release_month": release_month,
        "cover_url": r.cover_url,
        "confidence": r.confidence,
        "primary_artist": {
            "id": r.primary_artist_id.unwrap_or_else(uuid::Uuid::nil),
            "name": r.primary_artist_name.unwrap_or_default(),
            "avatar_url": r.primary_artist_avatar,
        },
        "track_count": r.track_count,
        "total_duration_ms": r.total_duration_ms,
        "popularity": r.popularity_score,
        "star": r.is_star_artist,
    })
}

#[cfg(test)]
pub(crate) mod testing {
    use super::SearchService;
    use crate::error::AppResult;

    pub(crate) async fn expensive_once<Load, LoadFuture>(
        service: &SearchService,
        key: &str,
        compute: Load,
    ) -> AppResult<u32>
    where
        Load: FnOnce() -> LoadFuture,
        LoadFuture: std::future::Future<Output = AppResult<u32>>,
    {
        let value = service
            .cached(key, || async { Ok(serde_json::json!(compute().await?)) })
            .await?;
        Ok(value.as_u64().unwrap_or_default() as u32)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cached_page_cannot_reopen_pagination_past_the_limit() {
        for page in [23, 24] {
            let result = super::decode_page(
                serde_json::json!({
                    "collection": [{"id": "cached-artist"}], "page": page,
                    "page_size": 1, "has_more": true
                }),
                page,
                1,
            );
            assert_eq!(
                result.collection,
                vec![serde_json::json!({"id": "cached-artist"})]
            );
            assert_eq!(
                (result.page, result.page_size, result.has_more),
                (page, 1, page < 24)
            );
        }
    }
}
