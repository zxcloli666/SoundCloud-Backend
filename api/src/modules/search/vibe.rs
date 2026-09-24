use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::semantic::{CacheHitPolicy, Cacheable, VibeSearchService, sc_id_of_track, sha_key};
use crate::error::AppResult;
use crate::modules::enrich::dto as enrich_dto;

const VIBE_RES_TTL_SECS: u64 = 90;

const VIBE_MAX_LIMIT: usize = 40;
const VIBE_DEFAULT_LIMIT: usize = 24;
const TOP_GENRES: usize = 3;

#[derive(Debug, Serialize, Deserialize)]
pub struct Atmosphere {
    #[serde(rename = "topGenres")]
    pub top_genres: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VibeResponse {
    pub items: Vec<Value>,
    pub atmosphere: Atmosphere,
    pub status: String,
}

impl VibeSearchService {
    pub async fn vibe(
        &self,
        q: &str,
        limit: Option<usize>,
        languages: Option<&[String]>,
    ) -> AppResult<VibeResponse> {
        let Some(q_norm) = Self::normalize_query(q) else {
            return Ok(empty_vibe());
        };
        let limit = limit.unwrap_or(VIBE_DEFAULT_LIMIT).clamp(1, VIBE_MAX_LIMIT);
        let lang_key = languages.map(|l| l.join(",")).unwrap_or_default();
        let key = vibe_res_key(&q_norm, limit, &lang_key);

        self.cached_typed(
            &key,
            VIBE_RES_TTL_SECS,
            CacheHitPolicy::Public(vibe_cache_track_ids),
            || async {
                let st = self
                    .recommendations
                    .search_by_text(&q_norm, limit, languages)
                    .await?;
                if st.preparing {
                    return Ok(Cacheable::skip(preparing_vibe()));
                }
                if st.failed {
                    return Ok(Cacheable::skip(empty_vibe()));
                }

                let top_genres = top_genres_of(&st.results, TOP_GENRES);
                let sc_ids: Vec<String> = st
                    .results
                    .iter()
                    .map(|r| crate::modules::recommendations::value_id_to_string(&r.id))
                    .collect();
                let mut items = self.project_ordered(&sc_ids).await?;
                enrich_dto::apply_to_tracks(&self.pg, &mut items).await?;

                Ok(Cacheable::keep(VibeResponse {
                    items,
                    atmosphere: Atmosphere { top_genres },
                    status: "ready".into(),
                }))
            },
        )
        .await
    }
}

pub(super) fn vibe_cache_track_ids(response: &VibeResponse) -> Option<Vec<String>> {
    response.items.iter().map(sc_id_of_track).collect()
}

fn top_genres_of(
    items: &[crate::modules::recommendations::RecommendResult],
    n: usize,
) -> Vec<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for it in items {
        if let Some(g) = it.genre.as_ref() {
            let g = g.trim();
            if g.is_empty() {
                continue;
            }
            let key = g.to_string();
            let c = counts.entry(key.clone()).or_insert(0);
            if *c == 0 {
                order.push(key);
            }
            *c += 1;
        }
    }
    let mut ranked: Vec<(String, usize)> = order
        .into_iter()
        .map(|g| {
            let c = counts.get(&g).copied().unwrap_or(0);
            (g, c)
        })
        .collect();
    ranked.sort_by_key(|b| std::cmp::Reverse(b.1));
    ranked.into_iter().take(n).map(|(g, _)| g).collect()
}

pub(super) fn vibe_res_key(q: &str, limit: usize, languages: &str) -> String {
    sha_key("vibe:res:v2:", &[q, &limit.to_string(), languages])
}

fn empty_vibe() -> VibeResponse {
    VibeResponse {
        items: Vec::new(),
        atmosphere: Atmosphere {
            top_genres: Vec::new(),
        },
        status: "ready".into(),
    }
}

fn preparing_vibe() -> VibeResponse {
    VibeResponse {
        items: Vec::new(),
        atmosphere: Atmosphere {
            top_genres: Vec::new(),
        },
        status: "preparing".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn track(urn: &str) -> Value {
        json!({ "urn": urn })
    }

    #[test]
    fn a_result_carrying_an_unidentifiable_track_is_never_cached() {
        let complete = VibeResponse {
            items: vec![track("soundcloud:tracks:1"), track("soundcloud:tracks:2")],
            atmosphere: Atmosphere {
                top_genres: Vec::new(),
            },
            status: "ready".into(),
        };
        assert_eq!(
            vibe_cache_track_ids(&complete),
            Some(vec!["1".to_owned(), "2".to_owned()])
        );

        let broken = VibeResponse {
            items: vec![track("soundcloud:tracks:1"), json!({ "title": "no urn" })],
            atmosphere: Atmosphere {
                top_genres: Vec::new(),
            },
            status: "ready".into(),
        };
        assert_eq!(
            vibe_cache_track_ids(&broken),
            None,
            "one unidentifiable item must disqualify the whole answer, not be skipped"
        );
    }

    #[test]
    fn genres_are_ranked_by_weight_and_ties_keep_the_order_they_arrived_in() {
        use crate::modules::recommendations::RecommendResult;
        let item = |genre: Option<&str>| RecommendResult {
            id: json!("1"),
            score: None,
            payload: None,
            artist: None,
            genre: genre.map(str::to_owned),
            playback_count: None,
            features: None,
        };
        let items = vec![
            item(Some("house")),
            item(Some("techno")),
            item(Some("house")),
            item(Some("  ")),
            item(None),
            item(Some("ambient")),
            item(Some("techno")),
            item(Some("dub")),
        ];

        assert_eq!(
            top_genres_of(&items, TOP_GENRES),
            vec![
                "house".to_owned(),
                "techno".to_owned(),
                "ambient".to_owned()
            ],
            "blank and missing genres must not become entries, and equal counts keep arrival order"
        );
        assert!(top_genres_of(&items, 0).is_empty());
    }
}
