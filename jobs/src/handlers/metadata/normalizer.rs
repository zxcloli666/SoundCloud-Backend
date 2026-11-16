use std::collections::BTreeSet;

use regex::{Captures, Regex};
use unicode_normalization::UnicodeNormalization;
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

use super::phrases::{
    BRACKET_ONLY_MARKERS, MARKER_PHRASES, MARKER_WORDS, NOISE_PHRASES, VERSION_PHRASES,
    VERSION_WORDS,
};
use super::text::{casefold, unique, words};

const TIDY_EDGES: [char; 8] = [' ', '-', '–', '—', '|', '·', '•', ','];
const NOTE_EDGES: [char; 2] = ['.', ' '];

pub struct Normalizer {
    separator: Regex,
    bracketed: Regex,
    featured_inline: Regex,
    featured_note: Regex,
    produced: Regex,
    handle: Regex,
    name_split: Regex,
    whitespace: Regex,
    version_split: Regex,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TitleParts {
    pub artist: Option<String>,
    pub song: String,
    pub featured: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credits {
    pub main: Option<String>,
    pub featured: Vec<String>,
}

impl Normalizer {
    pub fn new() -> Result<Self, regex::Error> {
        Ok(Self {
            separator: Regex::new(r"\s+[-–—−]\s+|\s+[|｜]\s+")?,
            bracketed: Regex::new(r"[(\[{]([^()\[\]{}]*)[)\]}]")?,
            featured_inline: Regex::new(r"(?i)\b(?:feat|ft|featuring)\b\.?\s*(.+)$")?,
            featured_note: Regex::new(r"(?i)^(?:feat|ft|featuring|with)\b\.?\s*(.+)$")?,
            produced: Regex::new(r"(?i)\b(?:prod|produced)\b\.?\s*(?:by\b)?\s*[.:]?\s*(.+)$")?,
            handle: Regex::new(r"[@#]\S+")?,
            name_split: Regex::new(r"(?i)\s*(?:,|&|\bx\b|\band\b|\+)\s*")?,
            whitespace: Regex::new(r"\s+")?,
            version_split: Regex::new(r"[\s+]+")?,
        })
    }

    pub fn parse_title(&self, title: &str) -> TitleParts {
        let (notes, bare) = self.strip_brackets(&self.tidy(&self.handle.replace_all(title, " ")));
        let (artist, song) = self.split_artist_and_song(&bare);
        let noted: Vec<String> = notes
            .iter()
            .filter_map(|note| self.featured_note.captures(note)?.get(1))
            .flat_map(|found| self.names(found.as_str()))
            .collect();
        let (artist, inline_featured) = self.cut(&self.featured_inline, &artist);
        let (song, song_featured) = self.cut(&self.featured_inline, &song);
        let (song, _) = self.cut(&self.produced, &song);
        TitleParts {
            artist: (!artist.is_empty()).then_some(artist),
            song,
            featured: unique(
                inline_featured
                    .into_iter()
                    .chain(song_featured)
                    .chain(noted),
            ),
        }
    }

    pub fn split_credits(&self, artist: &str) -> Credits {
        let (main, featured) = self.cut(&self.featured_inline, &self.tidy(artist));
        Credits {
            main: (!main.is_empty()).then_some(main),
            featured,
        }
    }

    pub fn version_markers(&self, title: &str) -> BTreeSet<&'static str> {
        let title_words = words(title);
        let mut found: BTreeSet<&'static str> = title_words
            .iter()
            .filter_map(|word| marker_of(MARKER_WORDS, word))
            .collect();
        for (first, second, marker) in MARKER_PHRASES {
            let said = title_words
                .windows(2)
                .any(|pair| matches!(pair, [left, right] if left == first && right == second));
            if said {
                found.insert(*marker);
            }
        }
        for note in self
            .bracketed
            .captures_iter(title)
            .filter_map(|found| found.get(1))
        {
            found.extend(
                words(note.as_str())
                    .iter()
                    .filter_map(|word| marker_of(BRACKET_ONLY_MARKERS, word)),
            );
        }
        found
    }

    pub fn tidy(&self, text: &str) -> String {
        let kept: String = text
            .nfkc()
            .filter(|character| !symbol_or_unassigned(*character))
            .collect();
        self.whitespace
            .replace_all(&kept, " ")
            .trim_matches(TIDY_EDGES)
            .to_owned()
    }

    fn strip_brackets(&self, text: &str) -> (Vec<String>, String) {
        let mut notes = Vec::new();
        let bare = self.bracketed.replace_all(text, |found: &Captures| {
            let inner = self.tidy(found.get(1).map_or("", |inner| inner.as_str()));
            if self.is_note(&inner) {
                notes.push(inner);
            }
            " "
        });
        let bare = self.tidy(&bare);
        (notes, bare)
    }

    fn is_note(&self, inner: &str) -> bool {
        let folded = casefold(inner);
        let folded = folded.trim_matches(NOTE_EDGES);
        let version = VERSION_PHRASES.contains(&folded)
            || self
                .version_split
                .split(folded)
                .any(|word| VERSION_WORDS.contains(&word));
        !inner.is_empty() && !NOISE_PHRASES.contains(&folded) && !version
    }

    fn split_artist_and_song(&self, text: &str) -> (String, String) {
        let parts: Vec<&str> = self
            .separator
            .split(text)
            .filter(|part| !part.trim().is_empty())
            .collect();
        match parts.split_first() {
            Some((artist, rest)) if !rest.is_empty() => {
                (self.tidy(artist), self.tidy(&rest.join(" ")))
            }
            _ => (String::new(), self.tidy(text)),
        }
    }

    fn cut(&self, pattern: &Regex, text: &str) -> (String, Vec<String>) {
        let found = pattern
            .captures(text)
            .and_then(|found| Some((found.get(0)?.start(), found.get(1)?.as_str())));
        match found {
            Some((start, named)) => (
                self.tidy(text.get(..start).unwrap_or_default()),
                self.names(named),
            ),
            None => (text.to_owned(), Vec::new()),
        }
    }

    fn names(&self, raw: &str) -> Vec<String> {
        self.name_split
            .split(raw)
            .map(|part| self.tidy(part))
            .filter(|name| !name.is_empty())
            .collect()
    }
}

fn marker_of(table: &[(&str, &'static str)], word: &str) -> Option<&'static str> {
    table
        .iter()
        .find(|(said, _)| *said == word)
        .map(|(_, marker)| *marker)
}

fn symbol_or_unassigned(character: char) -> bool {
    matches!(
        character.general_category(),
        GeneralCategory::OtherSymbol | GeneralCategory::Unassigned
    )
}
