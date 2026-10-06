use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::catalog::{DEFAULT_LIMIT, Request, SearchService};
use super::terms::{self, Shape};
use crate::error::AppResult;
use crate::modules::enrich::dto as enrich_dto;
use crate::modules::tracks::repository::project_many_public;

const MODE: &str = "text";

#[derive(Debug, Serialize, Deserialize)]
pub struct LyricsHit {
    pub track: Value,
    #[serde(rename = "matchedLine")]
    pub matched_line: Option<String>,
    pub score: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LyricsSearchResponse {
    pub collection: Vec<LyricsHit>,
    pub page: i64,
    pub page_size: i64,
    pub has_more: bool,
    pub mode: String,
}

impl SearchService {
    pub async fn lyrics(
        &self,
        q: &str,
        page: Option<i64>,
        limit: Option<i64>,
    ) -> AppResult<LyricsSearchResponse> {
        let request = Request::new(q, page.unwrap_or(0), limit.unwrap_or(DEFAULT_LIMIT));
        let Some(q) = request.q.clone() else {
            return Ok(response(&request, Vec::new(), false));
        };
        let key = request.key("search-lyrics-v2", &q, None);
        self.cached(&key, || async {
            let mut tx = self.begin().await?;
            let Some(terms) = terms::resolve(&mut tx, &q, Shape::Lyrics).await? else {
                return Ok(response(&request, Vec::new(), false));
            };
            let rows = sqlx::query_file!(
                "queries/search/lyrics.sql",
                terms.strict,
                terms.loose,
                q,
                request.fetch(),
                request.offset()
            )
            .fetch_all(&mut *tx)
            .await?;
            tx.commit().await?;
            let (rows, more) = request.cut(rows);
            let ids: Vec<String> = rows.iter().map(|row| row.sc_track_id.clone()).collect();
            let projected = project_many_public(&self.pg, &ids).await?;
            let (mut tracks, matches): (Vec<Value>, Vec<(String, f64)>) = rows
                .into_iter()
                .zip(projected)
                .filter_map(|(row, track)| {
                    let score = f64::from(row.score).clamp(0.0, 1.0);
                    Some((track?, (row.matched_line, score)))
                })
                .unzip();
            enrich_dto::apply_to_tracks(&self.pg, &mut tracks).await?;
            let collection = tracks
                .into_iter()
                .zip(matches)
                .map(|(track, (matched_line, score))| LyricsHit {
                    track,
                    matched_line: Some(matched_line),
                    score,
                })
                .collect();
            Ok(response(&request, collection, more))
        })
        .await
    }
}

fn response(request: &Request, collection: Vec<LyricsHit>, has_more: bool) -> LyricsSearchResponse {
    LyricsSearchResponse {
        collection,
        page: request.page,
        page_size: request.limit,
        has_more,
        mode: MODE.to_owned(),
    }
}
