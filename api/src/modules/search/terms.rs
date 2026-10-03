use catalog_normalize::{VersionMarker, cyrillic_to_latin, normalize_name, title_forms};
use sha2::{Digest, Sha256};

const MAX_TOKENS: usize = 8;
const MAX_SLOTS: usize = 4;
const SLOT_MIN_CHARS: usize = 3;
const MAX_SPAN_TOKENS: usize = 4;
const MAX_SPLIT_TOKENS: usize = 2;
const MAX_SPLITS: usize = 5;
const PREFIX_ONLY_BELOW: usize = 3;

const MARKER_WORDS: [(&str, VersionMarker); 14] = [
    ("remix", VersionMarker::Remix),
    ("rmx", VersionMarker::Remix),
    ("sped", VersionMarker::SpedUp),
    ("spedup", VersionMarker::SpedUp),
    ("speed", VersionMarker::SpedUp),
    ("nightcore", VersionMarker::SpedUp),
    ("slowed", VersionMarker::Slowed),
    ("reverb", VersionMarker::Reverb),
    ("cover", VersionMarker::Cover),
    ("instrumental", VersionMarker::Instrumental),
    ("live", VersionMarker::Live),
    ("mashup", VersionMarker::Mashup),
    ("extended", VersionMarker::Extended),
    ("acoustic", VersionMarker::Acoustic),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub start: usize,
    pub len: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Split {
    pub artist: String,
    pub title: String,
}

#[derive(Clone, Debug)]
pub struct QueryTerms {
    pub norm: String,
    pub tokens: Vec<String>,
    pub slots: Vec<String>,
    pub spans: Vec<Span>,
    pub markers: Vec<VersionMarker>,
    pub prefix_only: bool,
}

impl QueryTerms {
    pub fn parse(raw: &str) -> Self {
        let norm = normalize_name(raw);
        let tokens: Vec<String> = norm
            .split_whitespace()
            .take(MAX_TOKENS)
            .map(str::to_owned)
            .collect();
        Self {
            slots: slots_of(tokens.iter().map(String::as_str)),
            spans: spans_of(&tokens),
            markers: markers_of(raw, &tokens),
            prefix_only: norm.chars().count() < PREFIX_ONLY_BELOW,
            norm,
            tokens,
        }
    }

    pub fn is_single(&self) -> bool {
        self.tokens.len() <= 1
    }

    pub fn latin(&self) -> Option<Self> {
        cyrillic_to_latin(&self.norm).map(|latin| Self::parse(&latin))
    }

    pub fn hash(&self) -> String {
        hex::encode(Sha256::digest(self.norm.as_bytes()))[..32].to_owned()
    }

    pub fn span_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.spans.iter().map(|span| span.text.clone()).collect();
        names.sort();
        names.dedup();
        names
    }

    pub fn rest_slots(&self, span: &Span) -> Vec<String> {
        slots_of(
            self.tokens
                .iter()
                .enumerate()
                .filter(|(at, _)| *at < span.start || *at >= span.start + span.len)
                .map(|(_, token)| token.as_str()),
        )
    }

    pub fn splits(&self, span: Option<&Span>) -> Vec<Split> {
        let count = self.tokens.len();
        let mut splits: Vec<Split> = Vec::new();
        if let Some(span) = span {
            splits.push(self.split_at(span.start, span.len));
        }
        splits.push(Split {
            artist: String::new(),
            title: self.tokens.join(" "),
        });
        for len in 1..=MAX_SPLIT_TOKENS.min(count.saturating_sub(1)) {
            splits.push(self.split_at(0, len));
            splits.push(self.split_at(count - len, len));
        }
        let mut unique: Vec<Split> = Vec::new();
        for split in splits {
            if !unique.contains(&split) {
                unique.push(split);
            }
        }
        unique.truncate(MAX_SPLITS);
        unique
    }

    fn split_at(&self, start: usize, len: usize) -> Split {
        let end = (start + len).min(self.tokens.len());
        let rest: Vec<&str> = self.tokens[..start]
            .iter()
            .chain(&self.tokens[end..])
            .map(String::as_str)
            .collect();
        Split {
            artist: self.tokens[start..end].join(" "),
            title: rest.join(" "),
        }
    }
}

fn slots_of<'a>(tokens: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut long: Vec<&str> = tokens
        .filter(|token| token.chars().count() >= SLOT_MIN_CHARS)
        .collect();
    long.sort_by_key(|token| std::cmp::Reverse(token.chars().count()));
    let mut slots: Vec<String> = Vec::new();
    for token in long {
        let slot = format!("%{token}%");
        if slots.len() < MAX_SLOTS && !slots.contains(&slot) {
            slots.push(slot);
        }
    }
    slots
}

fn spans_of(tokens: &[String]) -> Vec<Span> {
    let count = tokens.len();
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for len in 1..=MAX_SPAN_TOKENS.min(count) {
        ranges.push((0, len));
        ranges.push((count - len, len));
    }
    ranges.push((0, count));
    ranges.sort();
    ranges.dedup();
    let mut spans = Vec::new();
    for (start, len) in ranges.into_iter().filter(|(_, len)| *len > 0) {
        let text = tokens[start..start + len].join(" ");
        if let Some(latin) = cyrillic_to_latin(&text) {
            spans.push(Span {
                text: latin,
                start,
                len,
            });
        }
        spans.push(Span { text, start, len });
    }
    spans
}

fn markers_of(raw: &str, tokens: &[String]) -> Vec<VersionMarker> {
    let mut markers = title_forms(raw).markers;
    for token in tokens {
        for (word, marker) in MARKER_WORDS {
            if token == word && !markers.contains(&marker) {
                markers.push(marker);
            }
        }
    }
    markers
}
