use std::collections::{BTreeMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures::future::try_join_all;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};

use super::catalog::{Request, SearchService};
use super::failure::{self, Shed, search_timeout};
use super::lanes::Lane;
use super::terms::{self, Shape, Terms, quote};
use crate::error::{AppError, AppResult};

pub const MAX_VARIANTS: usize = 8;
const MAX_TOKENS: usize = 6;
const MIN_STEM_LEN: usize = 3;
pub const MAX_VARIANT_CHARS: usize = 80;
pub const DEFAULT_PER_KIND: i64 = 5;
pub const MAX_PER_KIND: i64 = 10;
pub const SOFT_BUDGET: Duration = Duration::from_millis(1800);
const WELL_KNOWN: f64 = 0.8;
const EXACT_ARTIST: f64 = 1.5;
const EXACT_TITLE: f64 = 1.0;

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

    fn settles(&self, known: bool) -> bool {
        let exact = match self.kind {
            Kind::Artist => EXACT_ARTIST,
            _ => EXACT_TITLE,
        };
        let trusted = known || self.popularity >= WELL_KNOWN;
        self.variant == 0 && self.tier == 0 && self.score >= exact && trusted
    }
}

struct Asked {
    variant: usize,
    q: String,
    spelled_too: bool,
    known: bool,
}

enum Answer {
    Found,
    Spelled(String),
}

struct Question<'a> {
    ask: &'a Asked,
    of_tracks: &'a Terms,
    of_entities: Option<&'a Terms>,
    stem: Option<&'a str>,
    per_kind: i64,
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

pub fn past_deadline(found: Vec<Resolved>) -> AppResult<Vec<Resolved>> {
    if found.is_empty() {
        Err(search_timeout())
    } else {
        Ok(found)
    }
}

pub fn keep_on_timeout(found: Vec<Resolved>, error: AppError) -> AppResult<Vec<Resolved>> {
    let error = failure::map(error);
    match Shed::of(&error) {
        Some(Shed::Timeout) => past_deadline(found),
        _ => Err(error),
    }
}

pub fn clean_kinds(kinds: &[Kind]) -> Vec<Kind> {
    if kinds.is_empty() {
        return ALL_KINDS.to_vec();
    }
    ALL_KINDS
        .into_iter()
        .filter(|kind| kinds.contains(kind))
        .collect()
}

pub fn first_kept(variants: &[String], queries: &[String]) -> bool {
    let first = variants.first().map(std::slice::from_ref);
    let first = first.map(clean_variants).unwrap_or_default();
    !first.is_empty() && first.first() == queries.first()
}

pub fn clean_variants(variants: &[String]) -> Vec<String> {
    let mut queries: Vec<String> = Vec::new();
    for raw in variants.iter().take(MAX_VARIANTS * 2) {
        let clipped: String = raw.chars().take(MAX_VARIANT_CHARS).collect();
        if let Some(q) = Request::new(&clipped, 0, 1).q
            && !queries.contains(&q)
        {
            queries.push(q);
        }
        if queries.len() == MAX_VARIANTS {
            break;
        }
    }
    queries
}

impl SearchService {
    pub async fn resolve(
        &self,
        variants: &[String],
        kinds: &[Kind],
        per_kind: i64,
        known: bool,
    ) -> AppResult<BTreeMap<Kind, Vec<Resolved>>> {
        let queries = clean_variants(variants);
        let known = known && first_kept(variants, &queries);
        let kinds = clean_kinds(kinds);
        let per_kind = per_kind.clamp(1, MAX_PER_KIND);
        let soft = Instant::now() + SOFT_BUDGET;
        let found = self
            .guarded(|| async {
                let mut lanes = self.lanes(kinds.len()).await?;
                let mut found = Vec::new();
                let mut asked: VecDeque<Asked> = queries
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(variant, q)| Asked {
                        variant,
                        q,
                        spelled_too: false,
                        known: known && variant == 0,
                    })
                    .collect();
                let mut seen: HashSet<String> = queries.iter().cloned().collect();
                while let Some(ask) = asked.pop_front() {
                    let left = soft.saturating_duration_since(Instant::now());
                    let answer = self.resolve_variant(&mut lanes, &ask, &kinds, per_kind);
                    let answer = tokio::time::timeout(left, answer).await;
                    let at = found.len();
                    found.extend(lanes.iter_mut().flat_map(|lane| lane.found.drain(..)));
                    match answer {
                        Ok(Ok(Answer::Found)) => {
                            if found[at..].iter().any(|hit| hit.settles(ask.known)) {
                                asked
                                    .retain(|next| next.spelled_too && next.variant == ask.variant);
                            }
                        }
                        Ok(Ok(Answer::Spelled(spelled))) => {
                            let variant = ask.variant;
                            let later = asked
                                .iter()
                                .position(|next| next.q == spelled && !next.spelled_too);
                            let moved_up = later.and_then(|at| asked.remove(at)).is_some();
                            asked.push_front(Asked {
                                spelled_too: true,
                                ..ask
                            });
                            if moved_up || seen.insert(spelled.clone()) {
                                asked.push_front(Asked {
                                    variant,
                                    q: spelled,
                                    spelled_too: false,
                                    known: false,
                                });
                            }
                        }
                        Ok(Err(error)) => return keep_on_timeout(found, error),
                        Err(_) => return past_deadline(found),
                    }
                }
                try_join_all(lanes.into_iter().map(|lane| lane.tx.commit())).await?;
                Ok::<Vec<Resolved>, crate::error::AppError>(found)
            })
            .await?;
        Ok(merge(found, per_kind as usize))
    }

    async fn resolve_variant(
        &self,
        lanes: &mut [Lane],
        ask: &Asked,
        kinds: &[Kind],
        per_kind: i64,
    ) -> AppResult<Answer> {
        let q = ask.q.as_str();
        let tx = &mut lanes[0].tx;
        let Some(of_tracks) = terms::resolve(&self.terms, tx, q, Shape::Tracks).await? else {
            return Ok(Answer::Found);
        };
        if !ask.spelled_too
            && let Some(spelled) = &of_tracks.spelled
        {
            return Ok(Answer::Spelled(spelled.clone()));
        }
        let of_entities = terms::resolve(&self.terms, tx, q, Shape::Entities).await?;
        let stem = match ask.spelled_too {
            true => None,
            false => stem_for(tx, q).await?,
        };
        let question = &Question {
            ask,
            of_tracks: &of_tracks,
            of_entities: of_entities.as_ref(),
            stem: stem.as_deref(),
            per_kind,
        };
        let next = &AtomicUsize::new(0);
        try_join_all(lanes.iter_mut().map(|lane| async move {
            while let Some(kind) = kinds.get(next.fetch_add(1, Ordering::Relaxed)) {
                let rows = question.rows(&mut lane.tx, *kind).await?;
                lane.found.extend(rows.into_iter().map(|row| Resolved {
                    id: row.id,
                    kind: *kind,
                    name: row.name,
                    artist: row.artist,
                    popularity: row.popularity,
                    score: row.match_score,
                    variant: ask.variant,
                    tier: row.tier,
                }));
            }
            Ok::<(), AppError>(())
        }))
        .await?;
        Ok(Answer::Found)
    }
}

impl Question<'_> {
    async fn rows(
        &self,
        tx: &mut Transaction<'static, Postgres>,
        kind: Kind,
    ) -> AppResult<Vec<Row>> {
        let terms = match (kind, self.of_entities) {
            (Kind::Track, _) => self.of_tracks,
            (_, Some(terms)) => terms,
            _ => return Ok(Vec::new()),
        };
        let q = self.ask.q.as_str();
        let strict = &terms.strict;
        let loose = terms.loose.as_deref().filter(|_| !self.ask.spelled_too);
        let rows = match kind {
            Kind::Track => {
                sqlx::query_file_as!(
                    Row,
                    "queries/search/resolve_tracks.sql",
                    strict,
                    loose,
                    q,
                    self.stem,
                    self.per_kind * 3
                )
                .fetch_all(&mut **tx)
                .await?
            }
            Kind::Artist => {
                sqlx::query_file_as!(
                    Row,
                    "queries/search/resolve_artists.sql",
                    strict,
                    loose,
                    q,
                    self.stem,
                    self.per_kind
                )
                .fetch_all(&mut **tx)
                .await?
            }
            Kind::Album => {
                sqlx::query_file_as!(
                    Row,
                    "queries/search/resolve_albums.sql",
                    strict,
                    loose,
                    q,
                    self.stem,
                    self.per_kind
                )
                .fetch_all(&mut **tx)
                .await?
            }
            Kind::Playlist => {
                sqlx::query_file_as!(
                    Row,
                    "queries/search/resolve_playlists.sql",
                    strict,
                    loose,
                    q,
                    self.stem,
                    self.per_kind
                )
                .fetch_all(&mut **tx)
                .await?
            }
        };
        Ok(rows)
    }
}

async fn stem_for(tx: &mut Transaction<'static, Postgres>, q: &str) -> AppResult<Option<String>> {
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
    fn past_the_deadline_what_was_found_is_kept() {
        let kept = past_deadline(vec![item(Kind::Track, "a", 0.9, 0, 0)]).unwrap();
        assert_eq!(kept.len(), 1);
        assert!(past_deadline(Vec::new()).is_err());
    }

    #[test]
    fn kinds_are_deduplicated_to_the_four_known_ones() {
        let many = vec![Kind::Track; 30_000];
        assert_eq!(clean_kinds(&many), vec![Kind::Track]);
        assert_eq!(
            clean_kinds(&[Kind::Playlist, Kind::Track, Kind::Playlist]),
            vec![Kind::Track, Kind::Playlist]
        );
        assert_eq!(clean_kinds(&[]), ALL_KINDS.to_vec());
    }

    #[test]
    fn variants_are_capped_deduplicated_and_clipped() {
        let long = "а".repeat(500);
        let mut raw: Vec<String> = (0..40).map(|n| format!("запрос {n}")).collect();
        raw.push(long);
        raw.insert(0, "Запрос 0".to_owned());
        let cleaned = clean_variants(&raw);
        assert_eq!(cleaned.len(), MAX_VARIANTS);
        assert!(
            cleaned
                .iter()
                .all(|q| q.chars().count() <= MAX_VARIANT_CHARS)
        );
        let unique: std::collections::HashSet<&String> = cleaned.iter().collect();
        assert_eq!(unique.len(), cleaned.len());
        let only_long = clean_variants(&["б".repeat(500)]);
        assert_eq!(only_long[0].chars().count(), MAX_VARIANT_CHARS);
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
