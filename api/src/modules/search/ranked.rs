use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

use catalog_normalize::NORMALIZER_VERSION;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::candidates;
use super::failure::{self, SearchFailure};
use super::rank;
use super::repository;
use super::terms::QueryTerms;
use crate::cache::cache_service::CacheScope;
use crate::cache::{CacheService, KeyedCoalesce, ListPageResult};
use crate::error::AppResult;
use crate::modules::enrich::dto as enrich_dto;
use crate::modules::tracks::TrackRow;

const MAX_IDS: usize = 300;
const LIST_TTL_SECONDS: u64 = 600;
const PARTIAL_TTL_SECONDS: u64 = 30;
const SEARCH_FIELD: &str = "_scd_search";

#[derive(Debug, Serialize, Deserialize)]
pub struct TrackPage {
    #[serde(flatten)]
    pub page: ListPageResult<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weak: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RankedList {
    ids: Vec<Uuid>,
    #[serde(default)]
    scores: Vec<f32>,
    weak: bool,
    partial: bool,
}

pub struct RankedSearch {
    pg: PgPool,
    cache: Arc<CacheService>,
    flights: KeyedCoalesce<Result<RankedList, SearchFailure>>,
}

impl RankedSearch {
    pub fn new(pg: PgPool, cache: Arc<CacheService>) -> Self {
        Self {
            pg,
            cache,
            flights: KeyedCoalesce::new(),
        }
    }

    pub async fn page(
        &self,
        raw: &str,
        page: i64,
        limit: i64,
        last_page: i64,
    ) -> AppResult<TrackPage> {
        let list = self.list(raw).await?;
        let total = list.ids.len();
        let start = usize::try_from(page.saturating_mul(limit))
            .unwrap_or(usize::MAX)
            .min(total);
        let end = start
            .saturating_add(usize::try_from(limit).unwrap_or(0))
            .min(total);
        let scores: HashMap<Uuid, f32> = list.ids[start..end]
            .iter()
            .copied()
            .zip(list.scores.iter().skip(start).copied())
            .collect();
        let rows = serving_rows(&self.pg, &list.ids[start..end]).await?;
        let served: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
        let mut collection = repository::project_tracks_with_uploaders(&self.pg, rows).await?;
        for (item, id) in collection.iter_mut().zip(&served) {
            if let Some(score) = scores.get(id) {
                set_search_field(item, "score", score_value(*score));
            }
        }
        enrich_dto::apply_to_tracks(&self.pg, &mut collection)
            .await
            .map_err(failure::from_app)?;
        Ok(TrackPage {
            page: ListPageResult {
                collection,
                page,
                page_size: limit,
                has_more: end < total && page < last_page,
            },
            weak: Some(list.weak),
        })
    }

    async fn list(&self, raw: &str) -> AppResult<RankedList> {
        let terms = QueryTerms::parse(raw);
        let key = format!("search:v2:t:{NORMALIZER_VERSION}:{}", terms.hash());
        if let Ok(Some(stored)) = self.cache.get_raw(&key).await
            && let Ok(list) = serde_json::from_str::<RankedList>(&stored)
        {
            return Ok(list);
        }
        let Ok(shared) = self
            .flights
            .run(&key, || async {
                Ok::<_, Infallible>(
                    self.rank(&key, raw, &terms)
                        .await
                        .map_err(SearchFailure::from_app),
                )
            })
            .await;
        shared.map_err(SearchFailure::into_app)
    }

    async fn rank(&self, key: &str, raw: &str, terms: &QueryTerms) -> AppResult<RankedList> {
        let pool = candidates::gather(&self.pg, raw, terms).await?;
        let ranking = rank::rank(terms, pool.span.as_ref(), pool.candidates);
        let (ids, scores) = ranking
            .scored
            .iter()
            .filter_map(|scored| Some((scored.key.parse::<Uuid>().ok()?, scored.score)))
            .take(MAX_IDS)
            .unzip();
        let list = RankedList {
            ids,
            scores,
            weak: ranking.weak(),
            partial: pool.partial,
        };
        let ttl = if list.partial {
            PARTIAL_TTL_SECONDS
        } else {
            LIST_TTL_SECONDS
        };
        if let Ok(body) = serde_json::to_string(&list) {
            let _ = self
                .cache
                .set_raw(key, &body, ttl, None, CacheScope::Shared, None)
                .await;
        }
        Ok(list)
    }
}

pub(super) async fn serving_rows(pg: &PgPool, ids: &[Uuid]) -> AppResult<Vec<TrackRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_file_as!(TrackRow, "queries/search/ranked/serving_by_ids.sql", ids)
        .fetch_all(pg)
        .await
        .map_err(failure::from_db)
}

pub fn set_search_field(item: &mut Value, key: &str, value: Value) {
    let Some(object) = item.as_object_mut() else {
        return;
    };
    let search = object.entry(SEARCH_FIELD).or_insert_with(|| json!({}));
    if !search.is_object() {
        *search = json!({});
    }
    if let Some(search) = search.as_object_mut() {
        search.insert(key.to_owned(), value);
    }
}

pub fn score_value(score: f32) -> Value {
    json!((f64::from(score) * 1000.0).round() / 1000.0)
}
