use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::wanted::{Wanted, clean_title, simplify};

const NEAR_MS: i64 = 15_000;
const MIN_PARTIAL_LEN: usize = 3;
const MIN_FUZZY_TITLE_LEN: usize = 3;

static VARIANT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(?:remix|sped up|speed up|slowed|nightcore|cover|8d|reverb|bass boosted|karaoke|mashup|instrumental|live)\b",
    )
    .expect("variant")
});

pub fn combined(wanted: &Wanted, hits: &[Value]) -> Option<String> {
    let fits: Vec<&Value> = hits.iter().filter(|hit| fits(wanted, hit)).collect();
    fits.iter()
        .find(|hit| uploaded_by(hit, &wanted.artist))
        .or(fits.first())
        .and_then(|hit| urn(hit))
}

pub fn by_uploader(wanted: &Wanted, hits: &[Value]) -> Option<String> {
    hits.iter()
        .find(|hit| fits(wanted, hit) && uploaded_by(hit, &wanted.artist))
        .and_then(urn)
}

fn urn(hit: &Value) -> Option<String> {
    hit["urn"].as_str().map(str::to_owned)
}

fn fits(wanted: &Wanted, hit: &Value) -> bool {
    close_duration(hit, wanted.duration_ms) && title_matches(wanted, hit)
}

fn close_duration(hit: &Value, wanted: Option<i64>) -> bool {
    let Some(want) = wanted else { return true };
    let have = hit["duration"].as_i64().unwrap_or(0);
    if have <= 0 {
        return true;
    }
    let diff = (have - want).abs();
    diff <= NEAR_MS || diff * 5 <= want
}

fn title_matches(wanted: &Wanted, hit: &Value) -> bool {
    let raw = hit["title"].as_str().unwrap_or_default();
    let want = wanted.title_key();
    let have = simplify(&clean_title(raw));
    if want.is_empty() || have.is_empty() {
        return false;
    }
    if VARIANT.is_match(raw) && !VARIANT.is_match(&wanted.raw_title) {
        return false;
    }
    let (short, long) = if want.len() <= have.len() {
        (&want, &have)
    } else {
        (&have, &want)
    };
    if short.chars().count() < MIN_FUZZY_TITLE_LEN {
        return short == long;
    }
    long.contains(short.as_str())
}

pub fn uploaded_by(hit: &Value, artist: &str) -> bool {
    let artist = simplify(artist);
    let uploader = simplify(hit["user"]["username"].as_str().unwrap_or_default());
    !artist.is_empty()
        && (uploader == artist
            || (artist.chars().count() >= MIN_PARTIAL_LEN && uploader.contains(&artist)))
}
