use serde_json::Value;
use sqlx::PgPool;

use crate::cache::ListPageResult;
use crate::error::AppResult;
use crate::modules::likes::cold as likes_cold;
use crate::modules::tracks::project_many_public;
use crate::qdrant::collections;

use super::service::RecommendationsService;
use super::service::util::{parse_id_or_null, value_to_u64};

const MAX_RELATED: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RelatedWindow {
    pub offset: usize,
    pub wanted: usize,
}

pub(crate) fn window(page: i64, limit: i64) -> Option<RelatedWindow> {
    let offset = usize::try_from(page.checked_mul(limit)?).ok()?;
    if offset >= MAX_RELATED {
        return None;
    }
    Some(RelatedWindow {
        offset,
        wanted: MAX_RELATED.min(offset + limit as usize + 1),
    })
}

pub(crate) async fn project_page(
    pg: &PgPool,
    sc_user_id: &str,
    ids: &[String],
    page: i64,
    limit: i64,
    has_more: bool,
) -> AppResult<ListPageResult<Value>> {
    let mut collection: Vec<Value> = project_many_public(pg, ids)
        .await?
        .into_iter()
        .flatten()
        .collect();
    likes_cold::apply_user_favorite_flag(pg, sc_user_id, &mut collection).await?;
    Ok(ListPageResult {
        collection,
        page,
        page_size: limit,
        has_more,
    })
}

impl RecommendationsService {
    pub async fn related_tracks(
        &self,
        sc_user_id: &str,
        sc_track_id: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<ListPageResult<Value>> {
        let page = page.clamp(0, 100);
        let limit = limit.clamp(1, 200);
        let empty = ListPageResult {
            collection: Vec::new(),
            page,
            page_size: limit,
            has_more: false,
        };
        let (Some(window), Some(anchor)) = (window(page, limit), parse_id_or_null(sc_track_id))
        else {
            return Ok(empty);
        };
        let neighbours = self.related_neighbours(anchor, window.wanted).await;
        let has_more = neighbours.len() > window.offset + limit as usize;
        let ids: Vec<String> = neighbours
            .into_iter()
            .skip(window.offset)
            .take(limit as usize)
            .collect();
        project_page(&self.pg, sc_user_id, &ids, page, limit, has_more).await
    }

    async fn related_neighbours(&self, anchor: u64, wanted: usize) -> Vec<String> {
        let seed = self.load_track_vectors(anchor).await;
        let filter = self.build_filter(&[anchor.to_string()], None);
        let arms = [
            (collections::TRACKS_COLLAB, seed.collab),
            (collections::TRACKS_MERT, seed.mert),
            (collections::TRACKS_CLAP, seed.clap),
        ];
        for (collection, vector) in arms {
            let Some(vector) = vector else { continue };
            let found = self
                .search_by_vector(collection, &vector, filter.as_ref(), wanted)
                .await;
            if !found.is_empty() {
                return found
                    .into_iter()
                    .filter_map(|result| value_to_u64(&result.id).map(|id| id.to_string()))
                    .collect();
            }
        }
        Vec::new()
    }
}

#[cfg(test)]
#[path = "related_tests.rs"]
mod tests;
