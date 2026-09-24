use std::collections::HashMap;

use backend_contracts::reasons::WorkerStatus;
use qdrant_client::qdrant::SearchPointsBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::debug;

use super::semantic::{CacheHitPolicy, Cacheable, VibeSearchService, sc_id_of_track, sha_key};
use crate::error::AppResult;
use crate::modules::enrich::dto as enrich_dto;
use crate::modules::lyrics::EncodeOutcome;
use crate::modules::tracks::{TrackRow, project_to_sc_shape};
use crate::qdrant::collections;

const LYRICS_RES_TTL_SECS: u64 = 60;

const LYRICS_MAX_LIMIT: i64 = 50;
const LYRICS_DEFAULT_LIMIT: i64 = 20;
const LYRICS_MAX_PAGE: i64 = 24;
const LYRICS_AUTO_MAX_WINDOW: i64 = 200;

const STATEMENT_TIMEOUT_MS: i32 = 2500;

const LYRICS_FTS_EXPR: &str = "lc.fts";

const ELIGIBILITY_OVERFETCH: usize = 4;
const LYRICS_SEARCH_MAX_POINTS: usize = 500;

fn nothing_will_ever_encode(outcome: &EncodeOutcome) -> bool {
    matches!(
        outcome,
        EncodeOutcome::Declined {
            status: WorkerStatus::Empty,
            ..
        }
    )
}

fn another_page_exists(more_rows: bool, page: i64) -> bool {
    more_rows && page < LYRICS_MAX_PAGE
}

fn mixed_has_more(merged: usize, arms_have_more: bool, page: i64, limit: i64) -> bool {
    let shown = (page + 1).saturating_mul(limit);
    if merged as i64 > shown {
        return true;
    }
    arms_have_more && shown < LYRICS_AUTO_MAX_WINDOW
}

fn points_to_fetch(offset: usize, limit: usize) -> usize {
    offset
        .saturating_add(limit)
        .saturating_add(1)
        .saturating_mul(ELIGIBILITY_OVERFETCH)
        .clamp(1, LYRICS_SEARCH_MAX_POINTS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsMode {
    Text,
    Semantic,
    Auto,
}

impl LyricsMode {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("text") => Self::Text,
            Some("semantic") => Self::Semantic,
            _ => Self::Auto,
        }
    }
    fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Semantic => "semantic",
            Self::Auto => "auto",
        }
    }
}

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

impl VibeSearchService {
    pub async fn lyrics(
        &self,
        q: &str,
        mode: LyricsMode,
        page: Option<i64>,
        limit: Option<i64>,
    ) -> AppResult<LyricsSearchResponse> {
        let page = page.unwrap_or(0).clamp(0, LYRICS_MAX_PAGE);
        let limit = limit
            .unwrap_or(LYRICS_DEFAULT_LIMIT)
            .clamp(1, LYRICS_MAX_LIMIT);
        let Some(q_norm) = Self::normalize_query(q) else {
            return Ok(empty_lyrics(page, limit, mode));
        };
        let key = lyrics_res_key(&q_norm, mode, page, limit);

        let policy = match mode {
            LyricsMode::Text => CacheHitPolicy::Public(lyrics_cache_track_ids),
            LyricsMode::Semantic => CacheHitPolicy::PublicLyricsVectors(lyrics_cache_track_ids),
            LyricsMode::Auto => CacheHitPolicy::Disabled,
        };
        self.cached_typed(&key, LYRICS_RES_TTL_SECS, policy, || async {
            let fetch = match mode {
                LyricsMode::Text => self.lyrics_text(&q_norm, page, limit).await?,
                LyricsMode::Semantic => self.lyrics_semantic(&q_norm, page, limit).await?,
                LyricsMode::Auto => self.lyrics_auto(&q_norm, page, limit).await?,
            };
            let collection = self.project_hit_tracks(fetch.hits).await?;
            let resp = LyricsSearchResponse {
                collection,
                page,
                page_size: limit,
                has_more: another_page_exists(fetch.has_more, page),
                mode: mode.as_str().to_string(),
            };
            Ok(Cacheable {
                value: resp,
                cache: fetch.cacheable,
            })
        })
        .await
    }

    async fn lyrics_text(&self, q: &str, page: i64, limit: i64) -> AppResult<LyricsFetch> {
        let offset = page * limit;
        let fetch_limit = limit + 1;

        let mut tx = self.pg.begin().await?;
        sqlx::query(&format!(
            "SET LOCAL statement_timeout = {STATEMENT_TIMEOUT_MS}"
        ))
        .execute(&mut *tx)
        .await?;

        let sql = format!(
            "SELECT lc.sc_track_id, \
                    ts_rank({expr}, websearch_to_tsquery('simple', $1)) AS rank, \
                    ts_headline('simple', \
                        coalesce(lc.plain_text, regexp_replace(coalesce(lc.synced_lrc, ''), '\\[[0-9:.]+\\]', ' ', 'g')), \
                        websearch_to_tsquery('simple', $1), \
                        'StartSel=<<, StopSel=>>, MaxFragments=1, MaxWords=14, MinWords=3, FragmentDelimiter= … ' \
                    ) AS matched \
             FROM lyrics_cache lc \
             JOIN tracks t ON t.sc_track_id = lc.sc_track_id \
             WHERE {expr} @@ websearch_to_tsquery('simple', $1) \
               AND t.sharing = 'public' \
               AND t.superseded_by IS NULL \
             ORDER BY rank DESC, lc.sc_track_id DESC \
             LIMIT $2 OFFSET $3",
            expr = LYRICS_FTS_EXPR
        );
        let rows: Vec<(String, f32, Option<String>)> = sqlx::query_as(&sql)
            .bind(q)
            .bind(fetch_limit)
            .bind(offset)
            .fetch_all(&mut *tx)
            .await?;

        tx.commit().await?;

        let has_more = rows.len() as i64 > limit;
        let hits = rows
            .into_iter()
            .take(limit as usize)
            .map(|(sc_track_id, rank, matched)| RawLyricsHit {
                sc_track_id,
                matched_line: matched
                    .map(|m| clean_headline(&m))
                    .filter(|s| !s.is_empty()),
                score: rank as f64,
            })
            .collect();
        Ok(LyricsFetch {
            hits,
            has_more,
            cacheable: true,
        })
    }

    async fn lyrics_semantic(&self, q: &str, page: i64, limit: i64) -> AppResult<LyricsFetch> {
        let vec = match self.worker.encode_lyrics_text(q).await? {
            EncodeOutcome::Ready(v) if !v.is_empty() => v,
            outcome => return Ok(LyricsFetch::empty(nothing_will_ever_encode(&outcome))),
        };
        let offset = (page * limit).max(0) as usize;
        let want = points_to_fetch(offset, limit as usize);

        let builder = SearchPointsBuilder::new(collections::TRACKS_LYRICS, vec, want as u64)
            .with_payload(true);
        let resp = match self.qdrant.raw().search_points(builder).await {
            Ok(r) => r,
            Err(e) => {
                debug!(error = %e, "lyrics semantic: qdrant search failed");
                return Ok(LyricsFetch::empty(false));
            }
        };

        let ids = resp
            .result
            .iter()
            .filter_map(|point| {
                let id = crate::modules::recommendations::point_id_to_value(point.id.clone());
                let sc_track_id = crate::modules::recommendations::value_id_to_string(&id);
                (!sc_track_id.is_empty() && sc_track_id != "null").then_some(sc_track_id)
            })
            .collect::<Vec<_>>();
        let (eligibility, public_ids) = tokio::join!(
            self.recommendations.lyrics_vector_eligibility(&ids),
            self.recommendations.public_track_ids(&ids),
        );
        let scored: Vec<RawLyricsHit> = resp
            .result
            .into_iter()
            .filter_map(|p| {
                let id = crate::modules::recommendations::point_id_to_value(p.id);
                let sc = crate::modules::recommendations::value_id_to_string(&id);
                if sc.is_empty()
                    || sc == "null"
                    || !public_ids.contains(&sc)
                    || !eligibility.matches(&sc, &p.payload)
                {
                    return None;
                }
                Some(RawLyricsHit {
                    sc_track_id: sc,
                    matched_line: None,
                    score: p.score as f64,
                })
            })
            .skip(offset)
            .collect();

        let has_more = scored.len() as i64 > limit;
        Ok(LyricsFetch {
            hits: scored.into_iter().take(limit as usize).collect(),
            has_more,
            cacheable: true,
        })
    }

    async fn lyrics_auto(&self, q: &str, page: i64, limit: i64) -> AppResult<LyricsFetch> {
        let full = ((page + 1) * limit).min(LYRICS_AUTO_MAX_WINDOW);
        let text = self.lyrics_text(q, 0, full).await?;
        let sem = self.lyrics_semantic(q, 0, full).await?;

        let arms_have_more = text.has_more || sem.has_more;
        let cacheable = text.cacheable && sem.cacheable;

        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut merged: Vec<RawLyricsHit> = Vec::with_capacity(full as usize + 1);
        for h in text.hits.into_iter().chain(sem.hits) {
            if seen.insert(h.sc_track_id.clone()) {
                merged.push(h);
            }
        }

        let start = (page * limit) as usize;
        let has_more = mixed_has_more(merged.len(), arms_have_more, page, limit);
        let pageful: Vec<RawLyricsHit> = merged
            .into_iter()
            .skip(start)
            .take(limit as usize)
            .collect();
        Ok(LyricsFetch {
            hits: pageful,
            has_more,
            cacheable,
        })
    }

    async fn project_hit_tracks(&self, hits: Vec<RawLyricsHit>) -> AppResult<Vec<LyricsHit>> {
        if hits.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<String> = hits.iter().map(|h| h.sc_track_id.clone()).collect();
        let rows: Vec<TrackRow> = sqlx::query_as(
            "SELECT * FROM tracks WHERE sc_track_id = ANY($1) AND sharing = 'public'",
        )
        .bind(&ids)
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

        let mut projected: HashMap<String, Value> = HashMap::with_capacity(by_id.len());
        for (id, row) in &by_id {
            let uploader = row
                .uploader_sc_user_id
                .as_deref()
                .and_then(|uid| users.get(uid));
            projected.insert(id.clone(), project_to_sc_shape(row, uploader));
        }
        let mut track_values: Vec<Value> = projected.into_values().collect();
        enrich_dto::apply_to_tracks(&self.pg, &mut track_values).await?;
        let mut by_sc: HashMap<String, Value> = HashMap::with_capacity(track_values.len());
        for tv in track_values {
            if let Some(id) = sc_id_of_track(&tv) {
                by_sc.insert(id, tv);
            }
        }

        Ok(hits
            .into_iter()
            .filter_map(|h| {
                by_sc.get(&h.sc_track_id).map(|tv| LyricsHit {
                    track: tv.clone(),
                    matched_line: h.matched_line,
                    score: h.score,
                })
            })
            .collect())
    }
}

struct LyricsFetch {
    hits: Vec<RawLyricsHit>,
    has_more: bool,
    cacheable: bool,
}

impl LyricsFetch {
    fn empty(cacheable: bool) -> Self {
        Self {
            hits: Vec::new(),
            has_more: false,
            cacheable,
        }
    }
}

#[derive(Debug, Clone)]
struct RawLyricsHit {
    sc_track_id: String,
    matched_line: Option<String>,
    score: f64,
}

fn lyrics_cache_track_ids(response: &LyricsSearchResponse) -> Option<Vec<String>> {
    response
        .collection
        .iter()
        .map(|hit| sc_id_of_track(&hit.track))
        .collect()
}

fn clean_headline(raw: &str) -> String {
    raw.replace("<<", "")
        .replace(">>", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn lyrics_res_key(q: &str, mode: LyricsMode, page: i64, limit: i64) -> String {
    sha_key(
        "lyrics:res:v4:",
        &[q, mode.as_str(), &page.to_string(), &limit.to_string()],
    )
}

fn empty_lyrics(page: i64, limit: i64, mode: LyricsMode) -> LyricsSearchResponse {
    LyricsSearchResponse {
        collection: Vec::new(),
        page,
        page_size: limit,
        has_more: false,
        mode: mode.as_str().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_an_empty_verdict_is_cached_as_no_hits() {
        assert!(nothing_will_ever_encode(&EncodeOutcome::Declined {
            status: WorkerStatus::Empty,
            reason: Some(backend_contracts::reasons::WorkerReason::EmptyText),
        }));
        assert!(
            !nothing_will_ever_encode(&EncodeOutcome::Declined {
                status: WorkerStatus::Failed,
                reason: Some(backend_contracts::reasons::WorkerReason::DeadlineExceeded),
            }),
            "a worker that ran out of time says nothing about the query; caching the miss \
             would hide the answer for the whole cache lifetime"
        );
        assert!(!nothing_will_ever_encode(&EncodeOutcome::Preparing));
    }

    #[test]
    fn the_last_reachable_page_does_not_promise_another_one() {
        assert!(another_page_exists(true, LYRICS_MAX_PAGE - 1));
        assert!(
            !another_page_exists(true, LYRICS_MAX_PAGE),
            "page {LYRICS_MAX_PAGE} is the last one a request can ask for, and a client that \
             follows has_more would ask for it again forever"
        );
        assert!(!another_page_exists(false, 0));
    }

    #[test]
    fn the_mixed_mode_admits_there_is_more_when_one_half_still_has_some() {
        assert!(
            mixed_has_more(20, true, 0, 20),
            "the words half found nothing and the vector half filled the page exactly; there \
             is more behind it and the client must be told"
        );
        assert!(mixed_has_more(21, false, 0, 20));
        assert!(!mixed_has_more(20, false, 0, 20));
    }

    #[test]
    fn the_mixed_mode_never_promises_a_page_beyond_its_own_window() {
        let limit = 20;
        let last_in_window = LYRICS_AUTO_MAX_WINDOW / limit - 1;
        assert!(
            !mixed_has_more(LYRICS_AUTO_MAX_WINDOW as usize, true, last_in_window, limit),
            "the mixed mode only ever looks at {LYRICS_AUTO_MAX_WINDOW} results, so it must \
             not promise a page it can never assemble"
        );
        assert!(mixed_has_more(
            LYRICS_AUTO_MAX_WINDOW as usize,
            true,
            0,
            limit
        ));
    }

    #[test]
    fn a_page_asks_for_more_points_than_it_shows_because_some_will_be_refused() {
        assert!(
            points_to_fetch(0, 5) > 6,
            "asking Qdrant for exactly one page means a single private neighbour makes the \
             page short, and the listener never learns there was more"
        );
        assert_eq!(points_to_fetch(0, 5), 24);
        assert_eq!(points_to_fetch(5, 5), 44);
    }

    #[test]
    fn a_deep_page_does_not_ask_qdrant_for_the_whole_collection() {
        assert_eq!(points_to_fetch(1_000_000, 50), LYRICS_SEARCH_MAX_POINTS);
        assert_eq!(
            points_to_fetch(usize::MAX, usize::MAX),
            LYRICS_SEARCH_MAX_POINTS
        );
    }

    #[test]
    fn the_smallest_page_still_asks_for_at_least_one_point() {
        assert!(points_to_fetch(0, 0) >= 1);
    }

    fn hit(urn: &str) -> LyricsHit {
        LyricsHit {
            track: json!({ "urn": urn }),
            matched_line: None,
            score: 1.0,
        }
    }

    #[test]
    fn an_unknown_mode_falls_back_to_auto_instead_of_guessing() {
        assert_eq!(LyricsMode::parse(Some("text")), LyricsMode::Text);
        assert_eq!(LyricsMode::parse(Some(" SEMANTIC ")), LyricsMode::Semantic);
        assert_eq!(LyricsMode::parse(Some("auto")), LyricsMode::Auto);
        assert_eq!(LyricsMode::parse(Some("nonsense")), LyricsMode::Auto);
        assert_eq!(LyricsMode::parse(Some("")), LyricsMode::Auto);
        assert_eq!(LyricsMode::parse(None), LyricsMode::Auto);
    }

    #[test]
    fn a_lyrics_answer_is_cached_only_when_every_hit_is_identifiable() {
        let complete = LyricsSearchResponse {
            collection: vec![hit("soundcloud:tracks:7"), hit("soundcloud:tracks:8")],
            page: 0,
            page_size: 20,
            has_more: false,
            mode: "text".into(),
        };
        assert_eq!(
            lyrics_cache_track_ids(&complete),
            Some(vec!["7".to_owned(), "8".to_owned()])
        );

        let mut broken = complete;
        broken.collection.push(LyricsHit {
            track: json!({ "urn": "not-a-track-urn" }),
            matched_line: None,
            score: 0.5,
        });
        assert_eq!(lyrics_cache_track_ids(&broken), None);
    }

    #[test]
    fn a_headline_loses_its_markers_and_its_ragged_spacing() {
        assert_eq!(clean_headline("  a <<burning>>\n  sky  "), "a burning sky");
        assert_eq!(clean_headline("<<>>"), "");
    }
}
