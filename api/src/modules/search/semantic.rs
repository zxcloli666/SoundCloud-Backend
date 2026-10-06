use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::cache::CacheService;
use crate::cache::cache_service::CacheScope;
use crate::error::AppResult;
use crate::modules::recommendations::RecommendationsService;
use crate::modules::tracks::{TrackRow, project_to_sc_shape};

pub(super) const MAX_QUERY_LEN: usize = 512;

pub(super) type CacheTrackIds<T> = fn(&T) -> Option<Vec<String>>;

pub struct VibeSearchService {
    pub(super) pg: PgPool,
    cache: Arc<CacheService>,
    pub(super) recommendations: Arc<RecommendationsService>,
    flights: crate::cache::KeyedCoalesce<String>,
}

impl VibeSearchService {
    pub fn new(
        pg: PgPool,
        cache: Arc<CacheService>,
        recommendations: Arc<RecommendationsService>,
    ) -> Arc<Self> {
        Arc::new(Self {
            pg,
            cache,
            recommendations,
            flights: crate::cache::KeyedCoalesce::new(),
        })
    }

    pub(super) fn normalize_query(raw: &str) -> Option<String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        let Some((cut_at, _)) = trimmed.char_indices().nth(MAX_QUERY_LEN) else {
            return Some(trimmed.to_owned());
        };
        let (kept, rest) = trimmed.split_at_checked(cut_at).unwrap_or((trimmed, ""));
        let whole_words = if rest.starts_with(char::is_whitespace) {
            kept
        } else {
            kept.rsplit_once(char::is_whitespace)
                .map_or(kept, |(words, _)| words)
        };
        Some(whole_words.trim_end().to_owned())
    }

    pub(super) async fn cached_typed<T, F, Fut>(
        &self,
        key: &str,
        ttl: u64,
        track_ids: CacheTrackIds<T>,
        compute: F,
    ) -> AppResult<T>
    where
        T: Serialize + serde::de::DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = AppResult<Cacheable<T>>>,
    {
        if let Ok(Some(raw)) = self.cache.get_raw(key).await
            && let Ok(v) = serde_json::from_str::<T>(&raw)
            && self.cache_hit_is_eligible(&v, track_ids).await
        {
            return Ok(v);
        }
        let json = self
            .flights
            .run(key, || async {
                let Cacheable { value, cache } = compute().await?;
                let json = serde_json::to_string(&value).unwrap_or_default();
                if cache && !json.is_empty() {
                    let _ = self
                        .cache
                        .set_raw(key, &json, ttl, None, CacheScope::Shared, None)
                        .await;
                }
                Ok::<String, crate::error::AppError>(json)
            })
            .await?;
        serde_json::from_str::<T>(&json)
            .map_err(|error| crate::error::AppError::internal(error.to_string()))
    }

    async fn cache_hit_is_eligible<T>(&self, value: &T, track_ids: CacheTrackIds<T>) -> bool {
        let Some(track_ids) = track_ids(value) else {
            return false;
        };
        self.recommendations
            .cached_tracks_still_eligible(&track_ids)
            .await
    }

    pub(super) async fn project_ordered(&self, ids: &[String]) -> AppResult<Vec<Value>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows: Vec<TrackRow> = sqlx::query_as(
            "SELECT * FROM tracks WHERE sc_track_id = ANY($1) AND sharing = 'public' AND deleted_at IS NULL",
        )
        .bind(ids)
        .fetch_all(&self.pg)
        .await?;
        let by_id: HashMap<String, TrackRow> = rows
            .into_iter()
            .map(|r| (r.sc_track_id.clone(), r))
            .collect();

        let uploader_ids: Vec<String> = by_id
            .values()
            .filter_map(|r| r.uploader_sc_user_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let users = self.load_uploaders(&uploader_ids).await?;

        Ok(ids
            .iter()
            .filter_map(|id| {
                by_id.get(id).map(|row| {
                    let uploader = row
                        .uploader_sc_user_id
                        .as_deref()
                        .and_then(|uid| users.get(uid));
                    project_to_sc_shape(row, uploader)
                })
            })
            .collect())
    }

    pub(super) async fn load_uploaders(&self, ids: &[String]) -> AppResult<HashMap<String, Value>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let users: Vec<crate::modules::users::UserRow> =
            sqlx::query_as("SELECT * FROM users WHERE sc_user_id = ANY($1)")
                .bind(ids)
                .fetch_all(&self.pg)
                .await?;
        Ok(users
            .into_iter()
            .map(|u| {
                (
                    u.sc_user_id.clone(),
                    crate::modules::users::project_to_sc_shape(&u),
                )
            })
            .collect())
    }
}

pub(super) struct Cacheable<T> {
    pub(super) value: T,
    pub(super) cache: bool,
}

impl<T> Cacheable<T> {
    pub(super) fn keep(value: T) -> Self {
        Self { value, cache: true }
    }
    pub(super) fn skip(value: T) -> Self {
        Self {
            value,
            cache: false,
        }
    }
}

pub(super) fn sc_id_of_track(t: &Value) -> Option<String> {
    if let Some(urn) = t.get("urn").and_then(|v| v.as_str()) {
        return crate::common::sc_ids::normalize_sc_track_id(urn);
    }
    None
}

pub(super) fn sha_key(prefix: &str, parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    format!("{prefix}{}", hex::encode(digest.finalize()))
}

#[cfg(test)]
pub(crate) mod testing {
    use super::{Cacheable, VibeSearchService};
    use crate::error::AppResult;

    pub(crate) async fn expensive_once<Load, LoadFuture>(
        service: &VibeSearchService,
        key: &str,
        compute: Load,
    ) -> AppResult<u32>
    where
        Load: FnOnce() -> LoadFuture,
        LoadFuture: std::future::Future<Output = AppResult<u32>>,
    {
        service
            .cached_typed(
                key,
                60,
                |_| None,
                || async { Ok(Cacheable::skip(compute().await?)) },
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::search::vibe::vibe_res_key;

    #[test]
    fn a_blank_query_is_no_query_and_a_long_one_is_cut_to_the_limit() {
        assert_eq!(VibeSearchService::normalize_query("  "), None);
        assert_eq!(VibeSearchService::normalize_query("\n\t"), None);
        assert_eq!(
            VibeSearchService::normalize_query("  midnight ocean  ").as_deref(),
            Some("midnight ocean")
        );
        let long = "я".repeat(MAX_QUERY_LEN + 50);
        let cut = VibeSearchService::normalize_query(&long).expect("a long query is still a query");
        assert_eq!(
            cut.chars().count(),
            MAX_QUERY_LEN,
            "the limit counts characters, not bytes"
        );
    }

    #[test]
    fn a_query_past_the_limit_is_cut_between_words() {
        let words = "love ".repeat(MAX_QUERY_LEN / 5 + 10);
        let cut = VibeSearchService::normalize_query(&format!("x{words}"))
            .expect("a long query is still a query");

        assert!(cut.chars().count() <= MAX_QUERY_LEN);
        assert!(
            cut.ends_with(" love"),
            "the last word was cut in half: {:?}",
            cut.rsplit(' ').next()
        );
    }

    #[test]
    fn cache_keys_separate_every_input_that_changes_the_answer() {
        let vibe = vibe_res_key("rain", 24);
        assert_ne!(vibe, vibe_res_key("rain", 25));
        assert_ne!(vibe, vibe_res_key("rains", 24));
        assert!(vibe.starts_with("vibe:res:v3:"));
    }

    #[test]
    fn a_query_carrying_the_separator_cannot_steal_another_requests_answer() {
        assert_ne!(vibe_res_key("rain|24", 24), vibe_res_key("rain", 2424));
    }
}
