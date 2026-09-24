use catalog_normalize::{
    ParsedTitle, compact_title, name_similarity, normalize_name, normalize_title, parse_sc_title,
};
use serde_json::Value;

const SUBSTRING_MIN_CHARS: usize = 6;

#[derive(Debug, Clone)]
pub struct TrackMatch {
    pub title_score: f32,
    pub artist_score: f32,
    pub duration_match: DurationMatch,
    pub isrc_match: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationMatch {
    Exact,
    Close,
    Far,
    Unknown,
}

impl TrackMatch {
    pub fn score(&self) -> f32 {
        if self.isrc_match {
            return 1.0;
        }
        let duration = match self.duration_match {
            DurationMatch::Exact => 1.0,
            DurationMatch::Close => 0.7,
            DurationMatch::Unknown => 0.4,
            DurationMatch::Far => 0.0,
        };
        (self.title_score * 0.55 + self.artist_score * 0.35 + duration * 0.10).min(1.0)
    }
}

pub fn title_score(
    target_title: &str,
    candidate_title: &str,
    candidate_uploader: Option<&str>,
) -> f32 {
    let parsed = parse_sc_title(candidate_title, candidate_uploader);
    title_score_parsed(target_title, candidate_title, &parsed)
}

fn title_score_parsed(target_title: &str, candidate_title: &str, parsed: &ParsedTitle) -> f32 {
    let target = normalize_title(target_title);
    if target.is_empty() {
        return 0.0;
    }
    let target_compact = compact_title(target_title);

    let cleaned = normalize_title(&parsed.cleaned_title);
    let cleaned_compact = compact_title(&parsed.cleaned_title);
    let raw = normalize_title(candidate_title);
    let raw_compact = compact_title(candidate_title);

    if cleaned == target || raw == target {
        return 1.0;
    }
    if !target_compact.is_empty()
        && (cleaned_compact == target_compact || raw_compact == target_compact)
    {
        return 0.95;
    }

    if target_compact.chars().count() >= SUBSTRING_MIN_CHARS {
        if cleaned_compact.contains(&target_compact) || raw_compact.contains(&target_compact) {
            return 0.75;
        }
        if !cleaned_compact.is_empty()
            && target_compact.contains(&cleaned_compact)
            && cleaned_compact.chars().count() * 2 >= target_compact.chars().count()
        {
            return 0.65;
        }
    }

    let trigrams = trigram_overlap(&cleaned_compact, &target_compact)
        .max(trigram_overlap(&raw_compact, &target_compact));
    if trigrams >= 0.7 {
        return 0.55;
    }
    if trigrams >= 0.5 {
        return 0.4;
    }
    0.0
}

pub fn artist_score(
    target_artist: &str,
    candidate_uploader: Option<&str>,
    candidate_title_artist: Option<&str>,
) -> f32 {
    if normalize_name(target_artist).is_empty() {
        return 0.5;
    }
    let mut best = 0.0f32;
    for candidate in [candidate_title_artist, candidate_uploader]
        .into_iter()
        .flatten()
    {
        let similarity = name_similarity(target_artist, candidate);
        if similarity > best {
            best = similarity;
        }
    }
    best
}

pub fn duration_match(target_ms: Option<i32>, candidate_ms: Option<i64>) -> DurationMatch {
    let (Some(target), Some(candidate)) = (target_ms, candidate_ms) else {
        return DurationMatch::Unknown;
    };
    let difference = (candidate - target as i64).abs();
    if difference <= 1500 {
        DurationMatch::Exact
    } else if difference <= 5000 {
        DurationMatch::Close
    } else {
        DurationMatch::Far
    }
}

pub fn sc_track_id_from_urn(urn: &str) -> Option<String> {
    urn.rsplit(':')
        .next()
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

pub fn evaluate_sc_candidate(
    candidate: &Value,
    wanted_title: &str,
    wanted_artist: &str,
    wanted_isrc: Option<&str>,
    wanted_duration_ms: Option<i32>,
) -> TrackMatch {
    let title = candidate.get("title").and_then(Value::as_str).unwrap_or("");
    let uploader = candidate
        .get("user")
        .and_then(|user| user.get("username"))
        .and_then(Value::as_str);
    let isrc = candidate
        .pointer("/publisher_metadata/isrc")
        .and_then(Value::as_str);
    let duration_ms = candidate.get("duration").and_then(Value::as_i64);

    let parsed = parse_sc_title(title, uploader);
    let parsed_artist = parsed.primary_artists.first().map(String::as_str);

    TrackMatch {
        title_score: title_score_parsed(wanted_title, title, &parsed),
        artist_score: artist_score(wanted_artist, uploader, parsed_artist),
        duration_match: duration_match(wanted_duration_ms, duration_ms),
        isrc_match: matches!((wanted_isrc, isrc), (Some(ours), Some(theirs)) if ours.eq_ignore_ascii_case(theirs)),
    }
}

fn ngram_set(value: &str, size: usize) -> std::collections::HashSet<String> {
    let characters: Vec<char> = value.chars().collect();
    if characters.len() < size {
        return std::collections::HashSet::new();
    }
    let mut set = std::collections::HashSet::with_capacity(characters.len() - size + 1);
    for window in characters.windows(size) {
        set.insert(window.iter().collect::<String>());
    }
    set
}

fn trigram_overlap(left: &str, right: &str) -> f32 {
    let left = ngram_set(left, 3);
    let right = ngram_set(right, 3);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(&right).count() as f32;
    let union = left.union(&right).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_title_wins_through_the_artist_prefix() {
        assert!((title_score("Lose Yourself", "Eminem - Lose Yourself", None) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn punctuation_only_difference_still_matches() {
        let score = title_score(
            "1000-7?что ты сказал?",
            "Psychosis, Pxlsdead - 1000 - 7что Ты Сказал",
            None,
        );

        assert!(score >= 0.95, "expected a compact match, got {score}");
    }

    #[test]
    fn short_target_inside_a_longer_candidate_matches() {
        let score = title_score("100-7", "psychosis - 100-7 (slowed reverb)", None);

        assert!(score >= 0.75, "expected a substring match, got {score}");
    }

    #[test]
    fn unrelated_titles_score_low() {
        let score = title_score("totally different", "another song completely", None);

        assert!(score < 0.4, "expected a low score, got {score}");
    }

    #[test]
    fn uploader_name_identifies_the_artist() {
        assert!((artist_score("Ultimathule", Some("ultimathule"), None) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn country_suffix_does_not_break_the_artist_match() {
        let score = artist_score("ultimathule (RUS)", Some("ultimathule"), None);

        assert!(score >= 0.85, "expected a substring match, got {score}");
    }

    #[test]
    fn a_wanted_track_without_an_artist_stays_neutral() {
        assert!((artist_score("", Some("anyone"), None) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_reuploader_never_passes_as_the_artist() {
        let score = artist_score("Drake", Some("RandomReuploader42"), None);

        assert!(
            score < 0.4,
            "an unrelated uploader must score low, got {score}"
        );
    }

    #[test]
    fn duration_falls_into_the_expected_buckets() {
        assert_eq!(
            duration_match(Some(180_000), Some(180_500)),
            DurationMatch::Exact
        );
        assert_eq!(
            duration_match(Some(180_000), Some(183_000)),
            DurationMatch::Close
        );
        assert_eq!(
            duration_match(Some(180_000), Some(220_000)),
            DurationMatch::Far
        );
        assert_eq!(duration_match(None, Some(180_000)), DurationMatch::Unknown);
    }

    #[test]
    fn a_matching_isrc_pins_the_score_to_one() {
        let matched = TrackMatch {
            title_score: 0.0,
            artist_score: 0.0,
            duration_match: DurationMatch::Far,
            isrc_match: true,
        };

        assert!((matched.score() - 1.0).abs() < 1e-6);
    }
}
