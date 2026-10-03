use std::collections::{HashMap, HashSet};

use catalog_normalize::normalize_name;
use serde_json::{Value, json};

use super::query::{LiveKind, SIDE_ENOUGH_ROWS, TRACKS_ENOUGH_ROWS};
use super::slim::urn_of;
use crate::modules::search::rank::{Candidate, Ranking, rank};
use crate::modules::search::ranked::{score_value, set_search_field};
use crate::modules::search::terms::QueryTerms;

pub const SOURCE_SOUNDCLOUD: &str = "soundcloud";
pub const SOURCE_LOCAL: &str = "local";

pub fn wpages(window_len: usize, limit: i64) -> i64 {
    let limit = usize::try_from(limit.max(1)).unwrap_or(1);
    i64::try_from(window_len.div_ceil(limit)).unwrap_or(i64::MAX)
}

pub fn window_slice(ids: &[String], page: i64, limit: i64) -> &[String] {
    let limit = usize::try_from(limit.max(1)).unwrap_or(1);
    let start = usize::try_from(page.max(0))
        .unwrap_or(usize::MAX)
        .saturating_mul(limit);
    if start >= ids.len() {
        return &[];
    }
    &ids[start..(start + limit).min(ids.len())]
}

pub fn local_after_window(local: Vec<Value>, window_ids: &[String]) -> Vec<Value> {
    let known: HashSet<&str> = window_ids.iter().map(String::as_str).collect();
    local
        .into_iter()
        .filter(|row| urn_of(row).is_none_or(|urn| !known.contains(urn)))
        .collect()
}

pub fn substitute(items: Vec<Value>, serving: &HashMap<String, Option<Value>>) -> Vec<Value> {
    let mut out = Vec::with_capacity(items.len());
    for mut item in items {
        let found = urn_of(&item).and_then(|urn| serving.get(urn));
        match found {
            Some(Some(local)) => {
                let mut local = local.clone();
                tag_source(&mut local, SOURCE_LOCAL);
                out.push(local);
            }
            Some(None) => {}
            None => {
                tag_source(&mut item, SOURCE_SOUNDCLOUD);
                out.push(item);
            }
        }
    }
    dedupe_by_urn(out)
}

pub fn tag_source(item: &mut Value, source: &'static str) {
    set_search_field(item, "source", json!(source));
}

pub fn tag_all(items: &mut [Value], source: &'static str) {
    for item in items {
        tag_source(item, source);
    }
}

pub fn dedupe_by_urn(items: Vec<Value>) -> Vec<Value> {
    let mut seen: HashSet<String> = HashSet::new();
    items
        .into_iter()
        .filter(|item| match urn_of(item) {
            Some(urn) => seen.insert(urn.to_owned()),
            None => true,
        })
        .collect()
}

pub fn local_is_enough(kind: LiveKind, query: &str, rows: &[Value], ranked: bool) -> bool {
    match kind {
        LiveKind::Users | LiveKind::Playlists => rows.len() >= SIDE_ENOUGH_ROWS,
        LiveKind::Tracks if ranked => rows.len() >= TRACKS_ENOUGH_ROWS && confident(query, rows),
        LiveKind::Tracks => {
            let query_norm = normalize_name(query);
            rows.len() >= TRACKS_ENOUGH_ROWS
                && rows
                    .iter()
                    .take(TRACKS_ENOUGH_ROWS)
                    .any(|row| names_the_track(row, &query_norm))
        }
    }
}

fn names_the_track(row: &Value, query_norm: &str) -> bool {
    let Some(title) = row.get("title").and_then(Value::as_str) else {
        return false;
    };
    if normalize_name(title) == query_norm {
        return true;
    }
    let uploader = row.pointer("/user/username").and_then(Value::as_str);
    let artist = row
        .get("metadata_artist")
        .or_else(|| row.pointer("/publisher_metadata/artist"))
        .and_then(Value::as_str);
    [uploader, artist].into_iter().flatten().any(|name| {
        normalize_name(&format!("{name} {title}")) == query_norm
            || normalize_name(&format!("{title} {name}")) == query_norm
    })
}

pub fn confident(query: &str, rows: &[Value]) -> bool {
    ranking(query, rows, |_| None).confident()
}

pub fn mark_scores(query: &str, items: &mut [Value], window_offset: usize) {
    let scores: HashMap<String, f32> = ranking(query, items, |at| Some(window_offset + at))
        .scored
        .into_iter()
        .map(|scored| (scored.key, scored.score))
        .collect();
    for item in items {
        if let Some(score) = urn_of(item).and_then(|urn| scores.get(urn)) {
            set_search_field(item, "score", score_value(*score));
        }
    }
}

pub fn confident_pick(query: &str, hits: Vec<Value>, local: Vec<Value>) -> Option<Value> {
    let hits = dedupe_by_urn(hits);
    let live = hits.len();
    let known: HashSet<String> = hits
        .iter()
        .filter_map(|item| urn_of(item).map(str::to_owned))
        .collect();
    let mut items = hits;
    items.extend(
        local
            .into_iter()
            .filter(|row| urn_of(row).is_none_or(|urn| !known.contains(urn))),
    );
    let ranked = ranking(query, &items, |at| (at < live).then_some(at));
    if !ranked.confident() {
        return None;
    }
    let top = ranked.top()?;
    let mut pick = items
        .into_iter()
        .find(|item| urn_of(item) == Some(top.key.as_str()))?;
    set_search_field(&mut pick, "score", score_value(top.score));
    Some(pick)
}

fn ranking(query: &str, items: &[Value], sc_rank: impl Fn(usize) -> Option<usize>) -> Ranking {
    let candidates = items
        .iter()
        .enumerate()
        .filter_map(|(at, item)| {
            let mut candidate = Candidate::from_item(item)?;
            candidate.sc_rank = sc_rank(at);
            Some(candidate)
        })
        .collect();
    rank(&QueryTerms::parse(query), None, candidates)
}

pub fn is_live(item: &Value) -> bool {
    item.pointer("/_scd_search/source").and_then(Value::as_str) == Some(SOURCE_SOUNDCLOUD)
}
