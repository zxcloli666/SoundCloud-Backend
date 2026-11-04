use std::sync::LazyLock;

use regex::Regex;

static ARTIST_SPLIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*,\s*|\s*&\s*|\s+feat\b|\s+ft\.|\s+ft\s|\s+x\s").expect("artist split")
});
static CHANNEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s*(?:-\s*topic|vevo)\s*$").expect("channel"));
static NOISE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\s*[(\[【][^)\]】]*\b(?:official|video|audio|lyrics?|visuali[sz]er|mv|m/v|hd|hq|4k|clip|клип|премьера|текст|remaster(?:ed)?|feat|ft|featuring|prod|produced)\b[^)\]】]*[)\]】]",
    )
    .expect("noise")
});
static REMASTER_TAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s+[-–—]\s+(?:\d{4}\s+)?remaster(?:ed)?\b.*$").expect("remaster tail")
});
static PIPE_TAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*[|｜].*$").expect("pipe tail"));
static FEAT_TAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\s+(?:feat\.?|ft\.|featuring)\s+.*$").expect("feat tail"));
static BRACKETS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*[(\[【][^)\]】]*[)\]】]").expect("brackets"));

#[derive(Debug, Clone)]
pub struct Wanted {
    pub artist: String,
    pub title: String,
    pub raw_title: String,
    pub duration_ms: Option<i64>,
}

pub type Key = (String, String, Option<i64>);

impl Wanted {
    pub fn new(artist: &str, title: &str, duration_ms: Option<i64>) -> Self {
        Self {
            artist: first_artist(artist),
            title: clean_title(title),
            raw_title: title.trim().to_owned(),
            duration_ms: duration_ms.filter(|ms| *ms > 0),
        }
    }

    pub fn artist_key(&self) -> String {
        simplify(&self.artist)
    }

    pub fn title_key(&self) -> String {
        simplify(&self.title)
    }

    pub fn key(&self) -> Key {
        (self.artist_key(), self.title_key(), self.duration_ms)
    }
}

pub fn first_artist(raw: &str) -> String {
    let channel = CHANNEL.replace(raw.trim(), "");
    ARTIST_SPLIT
        .split(&channel)
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned()
}

pub fn clean_title(raw: &str) -> String {
    let title = NOISE.replace_all(raw.trim(), "");
    let title = REMASTER_TAIL.replace(&title, "");
    let title = PIPE_TAIL.replace(&title, "");
    let title = FEAT_TAIL.replace(&title, "");
    let title = title.trim();
    if title.is_empty() {
        raw.trim().to_owned()
    } else {
        title.to_owned()
    }
}

pub fn core_key(title: &str) -> String {
    simplify(&BRACKETS.replace_all(title, ""))
}

pub fn simplify(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
