use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};

use super::catalog::{Request, SearchService};
use super::terms::{self, Shape, quote};
use crate::error::AppResult;

const MAX_VARIANTS: usize = 8;
const MAX_TOKENS: usize = 6;
const MIN_STEM_LEN: usize = 3;
const PARALLEL_VARIANTS: usize = 3;
pub const DEFAULT_PER_KIND: i64 = 5;
pub const MAX_PER_KIND: i64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Track,
    Artist,
    Album,
    Playlist,
}

pub const ALL_KINDS: [Kind; 4] = [Kind::Track, Kind::Artist, Kind::Album, Kind::Playlist];

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Resolved {
    pub id: String,
    pub kind: Kind,
    pub name: String,
    pub artist: String,
    pub popularity: f64,
    pub score: f64,
    pub variant: usize,
    #[serde(skip)]
    tier: i32,
}

impl Resolved {
    fn rank(&self) -> f64 {
        self.score + 0.3 * self.popularity
    }

    fn beats(&self, other: &Resolved) -> bool {
        (self.tier, -self.rank()) < (other.tier, -other.rank())
    }
}

#[derive(Debug, sqlx::FromRow)]
struct Row {
    id: String,
    name: String,
    artist: String,
    popularity: f64,
    match_score: f64,
    tier: i32,
}

pub fn stem_query(pairs: &[(String, Option<String>)]) -> Option<String> {
    let mut changed = false;
    let clauses: Vec<String> = pairs
        .iter()
        .map(|(token, stem)| match stem {
            Some(stem) if stem != token && stem.chars().count() >= MIN_STEM_LEN => {
                changed = true;
                format!("{}:*", quote(stem))
            }
            _ => quote(token),
        })
        .collect();
    (changed && !clauses.is_empty()).then(|| clauses.join(" & "))
}

pub fn merge(found: Vec<Resolved>, per_kind: usize) -> BTreeMap<Kind, Vec<Resolved>> {
    let mut best: BTreeMap<(Kind, String), Resolved> = BTreeMap::new();
    for item in found {
        let key = (item.kind, item.id.clone());
        match best.get(&key) {
            Some(known) if !item.beats(known) => {}
            _ => {
                best.insert(key, item);
            }
        }
    }
    let mut grouped: BTreeMap<Kind, Vec<Resolved>> = BTreeMap::new();
    for (_, item) in best {
        grouped.entry(item.kind).or_default().push(item);
    }
    for items in grouped.values_mut() {
        items.sort_by(|a, b| {
            a.tier
                .cmp(&b.tier)
                .then(b.rank().total_cmp(&a.rank()))
                .then(a.id.cmp(&b.id))
        });
        items.truncate(per_kind);
    }
    grouped
}

impl SearchService {
    pub async fn resolve(
        &self,
        variants: &[String],
        kinds: &[Kind],
        per_kind: i64,
    ) -> AppResult<BTreeMap<Kind, Vec<Resolved>>> {
        let mut queries: Vec<String> = Vec::new();
        for raw in variants.iter().take(MAX_VARIANTS * 2) {
            if let Some(q) = Request::new(raw, 0, 1).q
                && !queries.contains(&q)
            {
                queries.push(q);
            }
            if queries.len() == MAX_VARIANTS {
                break;
            }
        }
        let per_kind = per_kind.clamp(1, MAX_PER_KIND);
        let found = self
            .guarded(|| async {
                let mut found = Vec::new();
                let numbered: Vec<(usize, &String)> = queries.iter().enumerate().collect();
                for chunk in numbered.chunks(PARALLEL_VARIANTS) {
                    let parts = futures::future::try_join_all(
                        chunk
                            .iter()
                            .map(|(index, q)| self.resolve_variant(*index, q, kinds, per_kind)),
                    )
                    .await?;
                    found.extend(parts.into_iter().flatten());
                }
                Ok::<Vec<Resolved>, crate::error::AppError>(found)
            })
            .await?;
        Ok(merge(found, per_kind as usize))
    }

    async fn resolve_variant(
        &self,
        index: usize,
        q: &str,
        kinds: &[Kind],
        per_kind: i64,
    ) -> AppResult<Vec<Resolved>> {
        let mut tx = self.begin().await?;
        let Some(strict_terms) = terms::resolve(&self.terms, &mut tx, q, Shape::Tracks).await?
        else {
            return Ok(Vec::new());
        };
        let entity_terms = terms::resolve(&self.terms, &mut tx, q, Shape::Entities).await?;
        let stem = stem_for(&mut tx, q).await?;
        let mut out = Vec::new();
        for kind in kinds {
            let (strict, loose) = match (kind, &entity_terms) {
                (Kind::Track, _) => (&strict_terms.strict, &strict_terms.loose),
                (_, Some(terms)) => (&terms.strict, &terms.loose),
                _ => continue,
            };
            let rows = match kind {
                Kind::Track => {
                    sqlx::query_file_as!(
                        Row,
                        "queries/search/resolve_tracks.sql",
                        strict,
                        loose.as_deref(),
                        q,
                        stem.as_deref(),
                        per_kind * 3
                    )
                    .fetch_all(&mut *tx)
                    .await?
                }
                Kind::Artist => {
                    sqlx::query_file_as!(
                        Row,
                        "queries/search/resolve_artists.sql",
                        strict,
                        loose.as_deref(),
                        q,
                        stem.as_deref(),
                        per_kind
                    )
                    .fetch_all(&mut *tx)
                    .await?
                }
                Kind::Album => {
                    sqlx::query_file_as!(
                        Row,
                        "queries/search/resolve_albums.sql",
                        strict,
                        loose.as_deref(),
                        q,
                        stem.as_deref(),
                        per_kind
                    )
                    .fetch_all(&mut *tx)
                    .await?
                }
                Kind::Playlist => {
                    sqlx::query_file_as!(
                        Row,
                        "queries/search/resolve_playlists.sql",
                        strict,
                        loose.as_deref(),
                        q,
                        stem.as_deref(),
                        per_kind
                    )
                    .fetch_all(&mut *tx)
                    .await?
                }
            };
            out.extend(rows.into_iter().map(|row| Resolved {
                id: row.id,
                kind: *kind,
                name: row.name,
                artist: row.artist,
                popularity: row.popularity,
                score: row.match_score,
                variant: index,
                tier: row.tier,
            }));
        }
        tx.commit().await?;
        Ok(out)
    }
}

async fn stem_for(tx: &mut Transaction<'_, Postgres>, q: &str) -> AppResult<Option<String>> {
    let tokens: Vec<String> = q
        .split_whitespace()
        .take(MAX_TOKENS)
        .map(str::to_owned)
        .collect();
    if tokens.is_empty() {
        return Ok(None);
    }
    let rows = sqlx::query_file!("queries/search/stems.sql", &tokens)
        .fetch_all(&mut **tx)
        .await?;
    let pairs: Vec<(String, Option<String>)> =
        rows.into_iter().map(|row| (row.token, row.stem)).collect();
    Ok(stem_query(&pairs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: Kind, id: &str, score: f64, tier: i32, variant: usize) -> Resolved {
        Resolved {
            id: id.to_owned(),
            kind,
            name: id.to_owned(),
            artist: String::new(),
            popularity: 0.0,
            score,
            variant,
            tier,
        }
    }

    #[test]
    fn stems_turn_case_forms_into_prefix_terms() {
        let pairs = vec![
            ("песни".to_owned(), Some("песн".to_owned())),
            ("земфиры".to_owned(), Some("земфир".to_owned())),
        ];
        assert_eq!(stem_query(&pairs).as_deref(), Some("'песн':* & 'земфир':*"));
    }

    #[test]
    fn nothing_changes_when_stems_equal_words() {
        let pairs = vec![
            ("кино".to_owned(), Some("кино".to_owned())),
            ("abc".to_owned(), None),
        ];
        assert_eq!(stem_query(&pairs), None);
    }

    #[test]
    fn a_short_stem_is_not_a_prefix() {
        let pairs = vec![
            ("он".to_owned(), Some("о".to_owned())),
            ("земфиры".to_owned(), Some("земфир".to_owned())),
        ];
        assert_eq!(stem_query(&pairs).as_deref(), Some("'он' & 'земфир':*"));
    }

    #[test]
    fn variants_merge_to_the_best_hit_per_entity() {
        let merged = merge(
            vec![
                item(Kind::Artist, "a", 0.4, 1, 0),
                item(Kind::Artist, "a", 0.9, 0, 1),
                item(Kind::Artist, "b", 0.95, 1, 0),
                item(Kind::Track, "t", 0.5, 0, 0),
            ],
            5,
        );
        let artists = &merged[&Kind::Artist];
        assert_eq!(artists.len(), 2);
        assert_eq!(artists[0].id, "a");
        assert_eq!(artists[0].variant, 1);
        assert_eq!(merged[&Kind::Track].len(), 1);
    }

    #[test]
    fn each_kind_is_cut_to_the_limit() {
        let many: Vec<Resolved> = (0..9)
            .map(|n| item(Kind::Track, &format!("t{n}"), 0.1 * n as f64, 0, 0))
            .collect();
        let merged = merge(many, 3);
        assert_eq!(merged[&Kind::Track].len(), 3);
        assert_eq!(merged[&Kind::Track][0].id, "t8");
    }
}
