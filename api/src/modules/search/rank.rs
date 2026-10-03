use std::cmp::Ordering;
use std::collections::HashSet;

use catalog_match::{artist_score, title_score};
use catalog_normalize::{
    ParsedTitle, VersionMarker, cyrillic_to_latin, normalize_name, parse_sc_title, same_artist,
    title_forms,
};
use serde_json::Value;

use super::terms::{QueryTerms, Span, Split};

pub const CONFIDENT_SCORE: f32 = 0.80;
const WEAK_SCORE: f32 = 0.65;
const STRONG_TITLE: f32 = 0.75;
const STRONG_TITLES_NEEDED: usize = 3;
const PREFIX_MATCH_CHARS: usize = 3;
const LIVE_WINDOW: f32 = 40.0;
const POPULARITY_SCALE: f64 = 18.4;
const MARKER_PENALTY: f32 = 0.15;
const MARKER_PENALTY_CAP: f32 = 0.30;
const PREVIEW_PENALTY: f32 = 0.25;
const JUNK_PENALTY: f32 = 0.20;

const TITLE_WEIGHT: f32 = 0.45;
const ARTIST_WEIGHT: f32 = 0.25;
const COVERAGE_WEIGHT: f32 = 0.15;
const POPULARITY_WEIGHT: f32 = 0.06;
const OFFICIAL_WEIGHT: f32 = 0.04;
const LIVE_RANK_WEIGHT: f32 = 0.05;

const DEMOTED_MARKERS: [VersionMarker; 9] = [
    VersionMarker::Remix,
    VersionMarker::SpedUp,
    VersionMarker::Slowed,
    VersionMarker::Reverb,
    VersionMarker::Cover,
    VersionMarker::Instrumental,
    VersionMarker::Live,
    VersionMarker::Mashup,
    VersionMarker::Extended,
];

const JUNK: [&str; 5] = ["8d", "bass boosted", "type beat", "1 hour", "lyrics"];

#[derive(Clone, Debug, Default)]
pub struct Candidate {
    pub key: String,
    pub title: String,
    pub uploader: Option<String>,
    pub metadata_artist: Option<String>,
    pub plays: i64,
    pub preview: bool,
    pub spanned: bool,
    pub linked: bool,
    pub sc_rank: Option<usize>,
}

impl Candidate {
    pub fn from_item(item: &Value) -> Option<Self> {
        let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
        Some(Self {
            key: text(item.get("urn"))?,
            title: text(item.get("title"))?,
            uploader: text(item.pointer("/user/username")),
            metadata_artist: text(
                item.get("metadata_artist")
                    .or_else(|| item.pointer("/publisher_metadata/artist")),
            ),
            plays: item
                .get("playback_count")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            preview: item.get("access").and_then(Value::as_str) == Some("preview"),
            ..Self::default()
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scored {
    pub key: String,
    pub score: f32,
    pub title: f32,
    pub artist: f32,
    pub coverage: f32,
    pub plays: i64,
}

#[derive(Clone, Debug, Default)]
pub struct Ranking {
    pub scored: Vec<Scored>,
}

impl Ranking {
    pub fn top(&self) -> Option<&Scored> {
        self.scored.first()
    }

    pub fn confident(&self) -> bool {
        self.top()
            .is_some_and(|top| top.score >= CONFIDENT_SCORE && top.coverage >= 1.0)
    }

    pub fn weak(&self) -> bool {
        let strong = self
            .scored
            .iter()
            .filter(|scored| scored.title >= STRONG_TITLE)
            .count();
        self.top().is_none_or(|top| top.score < WEAK_SCORE) || strong < STRONG_TITLES_NEEDED
    }
}

pub fn rank(terms: &QueryTerms, span: Option<&Span>, candidates: Vec<Candidate>) -> Ranking {
    let splits = terms.splits(span);
    let mut scored: Vec<Scored> = candidates
        .iter()
        .map(|candidate| score(terms, &splits, span, candidate))
        .collect();
    scored.sort_by(order);
    Ranking { scored }
}

fn score(
    terms: &QueryTerms,
    splits: &[Split],
    span: Option<&Span>,
    candidate: &Candidate,
) -> Scored {
    let parsed = parse_sc_title(&candidate.title, candidate.uploader.as_deref());
    let unsplit = Split::default();
    let (title, artist, split) = splits
        .iter()
        .map(|split| {
            let title = title_score(
                &split.title,
                &candidate.title,
                candidate.uploader.as_deref(),
            );
            let artist = if candidate.spanned {
                1.0
            } else {
                artist_of(&split.artist, candidate, &parsed)
            };
            (title, artist, split)
        })
        .max_by(|left, right| weigh(left.0, left.1).total_cmp(&weigh(right.0, right.1)))
        .unwrap_or((0.0, 0.0, &unsplit));
    let named = span.map_or(split.artist.as_str(), |span| span.text.as_str());
    let official = candidate.linked
        || (!named.is_empty()
            && candidate
                .uploader
                .as_deref()
                .is_some_and(|uploader| same_artist(uploader, named)));
    let coverage = coverage(&terms.tokens, candidate, &parsed);
    let live_rank = candidate
        .sc_rank
        .map_or(0.0, |rank| (1.0 - rank as f32 / LIVE_WINDOW).max(0.0));
    let score = weigh(title, artist)
        + COVERAGE_WEIGHT * coverage
        + POPULARITY_WEIGHT * popularity(candidate.plays)
        + OFFICIAL_WEIGHT * f32::from(u8::from(official))
        + LIVE_RANK_WEIGHT * live_rank
        - penalty(terms, candidate);
    Scored {
        key: candidate.key.clone(),
        score,
        title,
        artist,
        coverage,
        plays: candidate.plays,
    }
}

fn weigh(title: f32, artist: f32) -> f32 {
    TITLE_WEIGHT * title + ARTIST_WEIGHT * artist
}

fn artist_of(wanted: &str, candidate: &Candidate, parsed: &ParsedTitle) -> f32 {
    let credited = candidate
        .metadata_artist
        .as_deref()
        .or_else(|| parsed.primary_artists.first().map(String::as_str));
    let main = artist_score(wanted, candidate.uploader.as_deref(), credited);
    parsed
        .featured
        .iter()
        .map(|featured| artist_score(wanted, None, Some(featured)))
        .fold(main, f32::max)
}

fn coverage(tokens: &[String], candidate: &Candidate, parsed: &ParsedTitle) -> f32 {
    if tokens.is_empty() {
        return 0.0;
    }
    let texts = [
        Some(candidate.title.as_str()),
        candidate.uploader.as_deref(),
        candidate.metadata_artist.as_deref(),
    ];
    let mut words: HashSet<String> = HashSet::new();
    for text in texts
        .into_iter()
        .flatten()
        .chain(parsed.featured.iter().map(String::as_str))
    {
        let norm = normalize_name(text);
        if let Some(latin) = cyrillic_to_latin(&norm) {
            words.extend(latin.split_whitespace().map(str::to_owned));
        }
        words.extend(norm.split_whitespace().map(str::to_owned));
    }
    let found = tokens
        .iter()
        .filter(|token| {
            let latin = cyrillic_to_latin(token);
            [Some(token.as_str()), latin.as_deref()]
                .into_iter()
                .flatten()
                .any(|form| words.iter().any(|word| covers(word, form)))
        })
        .count();
    found as f32 / tokens.len() as f32
}

fn covers(word: &str, token: &str) -> bool {
    word == token || (token.chars().count() >= PREFIX_MATCH_CHARS && word.starts_with(token))
}

fn popularity(plays: i64) -> f32 {
    let scaled = ((1 + plays.max(0)) as f64).ln() / POPULARITY_SCALE;
    scaled.min(1.0) as f32
}

pub fn version_penalty(requested: &[VersionMarker], title: &str) -> f32 {
    let unrequested = title_forms(title)
        .markers
        .into_iter()
        .filter(|marker| DEMOTED_MARKERS.contains(marker) && !requested.contains(marker))
        .count();
    (unrequested as f32 * MARKER_PENALTY).min(MARKER_PENALTY_CAP)
}

fn penalty(terms: &QueryTerms, candidate: &Candidate) -> f32 {
    let markers = version_penalty(&terms.markers, &candidate.title);
    let preview = if candidate.preview {
        PREVIEW_PENALTY
    } else {
        0.0
    };
    let title = normalize_name(&candidate.title);
    let junk = if JUNK
        .iter()
        .any(|phrase| has_phrase(&title, phrase) && !has_phrase(&terms.norm, phrase))
    {
        JUNK_PENALTY
    } else {
        0.0
    };
    markers + preview + junk
}

fn has_phrase(text: &str, phrase: &str) -> bool {
    format!(" {text} ").contains(&format!(" {phrase} "))
}

fn order(left: &Scored, right: &Scored) -> Ordering {
    right
        .score
        .total_cmp(&left.score)
        .then(right.plays.cmp(&left.plays))
        .then_with(|| right.key.cmp(&left.key))
}
