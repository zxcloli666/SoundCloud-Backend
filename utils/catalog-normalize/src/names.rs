use once_cell::sync::Lazy;
use regex::Regex;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::title::{clean_artist_name, normalize_name};

fn fold_small_cap(c: char) -> Option<char> {
    Some(match c {
        '\u{1D00}' => 'a',
        '\u{0299}' => 'b',
        '\u{1D04}' => 'c',
        '\u{1D05}' => 'd',
        '\u{1D07}' => 'e',
        '\u{A730}' => 'f',
        '\u{0262}' => 'g',
        '\u{029C}' => 'h',
        '\u{026A}' => 'i',
        '\u{1D0A}' => 'j',
        '\u{1D0B}' => 'k',
        '\u{029F}' => 'l',
        '\u{1D0D}' => 'm',
        '\u{0274}' => 'n',
        '\u{1D0F}' => 'o',
        '\u{1D18}' => 'p',
        '\u{A7AF}' => 'q',
        '\u{0280}' => 'r',
        '\u{A731}' => 's',
        '\u{1D1B}' => 't',
        '\u{1D1C}' => 'u',
        '\u{1D20}' => 'v',
        '\u{1D21}' => 'w',
        '\u{028F}' => 'y',
        '\u{1D22}' => 'z',
        _ => return None,
    })
}

pub fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}' | '\u{FEFF}' | '\u{2060}' | '\u{FFFC}' | '\u{00AD}'
    )
}

pub fn fold_chars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.nfkd() {
        if is_combining_mark(c) || is_invisible(c) {
            continue;
        }
        let c = match c {
            '$' => 's',
            other => fold_small_cap(other).unwrap_or(other),
        };
        for lc in c.to_lowercase() {
            out.push(lc);
        }
    }
    out
}

pub fn unescape_json_unicode(s: &str) -> String {
    if !s.contains("\\u") {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        match parse_escape(&chars, i) {
            Some((cp, next)) => {
                if (0xD800..=0xDBFF).contains(&cp) {
                    if let Some((low, next2)) = parse_escape(&chars, next)
                        && (0xDC00..=0xDFFF).contains(&low)
                    {
                        let combined = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                        if let Some(c) = char::from_u32(combined) {
                            out.push(c);
                            i = next2;
                            continue;
                        }
                    }
                    out.push(chars[i]);
                    i += 1;
                } else if let Some(c) = char::from_u32(cp) {
                    out.push(c);
                    i = next;
                } else {
                    out.push(chars[i]);
                    i += 1;
                }
            }
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

fn parse_escape(chars: &[char], at: usize) -> Option<(u32, usize)> {
    if at + 6 > chars.len() || chars[at] != '\\' || chars[at + 1] != 'u' {
        return None;
    }
    let mut cp = 0u32;
    for &c in &chars[at + 2..at + 6] {
        cp = cp * 16 + c.to_digit(16)?;
    }
    Some((cp, at + 6))
}

static RE_SPLIT_NAMES: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\s*[,;]\s*|\s+(?:x|х|×|✕|✖|⨯|\+|vs\.?|&|and|w/|feat\.?|ft\.?|featuring|/)\s+")
        .unwrap()
});

static RE_JUNK_NAME: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?ix)
          :// | ^www\. | @gmail\. |
          \.(?:com|net|org|ru|ua|by|kz|de|fr|info|biz|me|tv|fm|cc|xyz|site|store|shop|blogspot)\b
        ",
    )
    .unwrap()
});

pub fn is_junk_artist_name(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || t.chars().count() > 100 {
        return true;
    }
    if RE_JUNK_NAME.is_match(t) {
        return true;
    }
    matches!(
        normalize_name(t).as_str(),
        "various artists"
            | "various"
            | "va"
            | "unknown"
            | "unknown artist"
            | "n a"
            | "none"
            | "null"
            | "no artist"
    )
}

pub fn split_artist_list(s: &str) -> Vec<String> {
    RE_SPLIT_NAMES
        .split(s)
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

pub fn meta_artist_names(meta: &str) -> Vec<String> {
    let unescaped = unescape_json_unicode(meta);
    let mut out: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for part in split_artist_list(&unescaped) {
        let cleaned = clean_artist_name(&part);
        if cleaned.is_empty() || is_junk_artist_name(&cleaned) {
            continue;
        }
        let key = normalize_name(&cleaned);
        if key.is_empty() || !seen.insert(key) {
            continue;
        }
        out.push(cleaned);
    }
    out
}

pub fn name_similarity(a: &str, b: &str) -> f32 {
    let an = normalize_name(a);
    let bn = normalize_name(b);
    if an.is_empty() || bn.is_empty() {
        return 0.0;
    }
    if an == bn {
        return 1.0;
    }
    let ac: String = an.chars().filter(|c| !c.is_whitespace()).collect();
    let bc: String = bn.chars().filter(|c| !c.is_whitespace()).collect();
    if ac == bc {
        return 0.95;
    }
    if transliterations_agree(&an, &bn) {
        return 0.95;
    }
    let (a_chars, b_chars) = (ac.chars().count(), bc.chars().count());
    if a_chars >= 4 && b_chars >= 4 && (ac.contains(&bc) || bc.contains(&ac)) {
        let short = a_chars.min(b_chars);
        let long = a_chars.max(b_chars);
        let (short_n, long_n) = if a_chars <= b_chars {
            (an.as_str(), bn.as_str())
        } else {
            (bn.as_str(), an.as_str())
        };
        if short * 2 >= long && word_aligned(short_n, long_n) {
            return 0.85;
        }
        return 0.55;
    }
    if bigram_overlap(&ac, &bc) >= 0.7 {
        return 0.5;
    }
    0.0
}

fn transliterations_agree(left: &str, right: &str) -> bool {
    let left_latin = crate::translit::cyrillic_to_latin(left);
    let right_latin = crate::translit::cyrillic_to_latin(right);
    if left_latin.is_none() && right_latin.is_none() {
        return false;
    }
    let left = left_latin.as_deref().unwrap_or(left);
    let right = right_latin.as_deref().unwrap_or(right);
    !left.is_empty() && compact_ascii(left) == compact_ascii(right)
}

fn compact_ascii(value: &str) -> String {
    value.chars().filter(|c| !c.is_whitespace()).collect()
}

fn word_aligned(short: &str, long: &str) -> bool {
    long.starts_with(&format!("{short} "))
        || long.ends_with(&format!(" {short}"))
        || long.contains(&format!(" {short} "))
}

pub fn same_artist(a: &str, b: &str) -> bool {
    name_similarity(a, b) >= 0.85
}

pub fn compact_key(s: &str) -> String {
    normalize_name(s)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

pub fn name_in<'a, I: IntoIterator<Item = &'a str>>(name: &str, set: I) -> bool {
    set.into_iter().any(|s| same_artist(name, s))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RawMetaMatch {
    Match,
    Partial,
    Mismatch,
}

pub fn compare_with_meta<'a, I>(detected: I, meta: &str) -> Option<RawMetaMatch>
where
    I: IntoIterator<Item = &'a str>,
{
    let raw = meta_artist_names(meta);
    if raw.is_empty() {
        return None;
    }
    let detected: Vec<&str> = detected
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .collect();
    if detected.is_empty() {
        return Some(RawMetaMatch::Mismatch);
    }
    let det_in_raw = detected
        .iter()
        .filter(|d| name_in(d, raw.iter().map(|s| s.as_str())))
        .count();
    if det_in_raw == 0 {
        return Some(RawMetaMatch::Mismatch);
    }
    let raw_in_det = raw
        .iter()
        .filter(|r| name_in(r, detected.iter().copied()))
        .count();
    if det_in_raw == detected.len() && raw_in_det == raw.len() {
        Some(RawMetaMatch::Match)
    } else {
        Some(RawMetaMatch::Partial)
    }
}

fn bigram_overlap(a: &str, b: &str) -> f32 {
    let sa = ngram_set(a);
    let sb = ngram_set(b);
    if sa.is_empty() || sb.is_empty() {
        return 0.0;
    }
    let inter = sa.intersection(&sb).count() as f32;
    let union = sa.union(&sb).count() as f32;
    inter / union
}

fn ngram_set(s: &str) -> std::collections::HashSet<[char; 2]> {
    let chars: Vec<char> = s.chars().collect();
    chars.windows(2).map(|w| [w[0], w[1]]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_small_caps_monarch() {
        assert_eq!(fold_chars("ᴍᴏɴᴀʀᴄʜ"), "monarch");
        assert!(same_artist("ᴍᴏɴᴀʀᴄʜ", "Monarch"));
    }

    #[test]
    fn transliterated_spelling_is_the_same_artist() {
        assert!(same_artist("МОКЕРИ", "Mokeri"));
        assert!(same_artist("Щука", "shchuka"));
        assert!(same_artist("без шансов", "Bez Shansov"));
    }

    #[test]
    fn transliteration_does_not_merge_different_artists() {
        assert!(!same_artist("Мокери", "Monarch"));
        assert!(!same_artist("Холод", "Cold"));
    }

    #[test]
    fn fold_math_and_fullwidth() {
        assert_eq!(fold_chars("𝐕𝐀𝐍𝐓𝐈"), "vanti");
        assert_eq!(fold_chars("ＦＵＬＬＷＩＤＴＨ"), "fullwidth");
    }

    #[test]
    fn fold_diacritics_both_alphabets() {
        assert_eq!(fold_chars("Françoise Hardy"), "francoise hardy");
        assert_eq!(fold_chars("Kanashī"), "kanashi");
        assert_eq!(fold_chars("Ёлка"), "елка");
    }

    #[test]
    fn unescape_basic_and_pairs() {
        assert_eq!(unescape_json_unicode(r"MARIO LONČARIĆ"), "MARIO LONČARIĆ");
        assert_eq!(unescape_json_unicode(r"Memento Mori 👽"), "Memento Mori 👽");
        assert_eq!(
            unescape_json_unicode(r"bad \uZZZZ tail"),
            r"bad \uZZZZ tail"
        );
        assert_eq!(
            unescape_json_unicode(r"lonely \uD83D end"),
            r"lonely \uD83D end"
        );
        assert_eq!(unescape_json_unicode("plain"), "plain");
    }

    #[test]
    fn meta_split_real_cases() {
        assert_eq!(
            meta_artist_names("Monarch, johnertekker"),
            vec!["Monarch", "johnertekker"]
        );
        assert_eq!(
            meta_artist_names("ghasaii, psychosis"),
            vec!["ghasaii", "psychosis"]
        );
        assert_eq!(
            meta_artist_names("E.P.O, Jorn L, Finnet, Turbo"),
            vec!["E.P.O", "Jorn L", "Finnet", "Turbo"]
        );
        assert_eq!(meta_artist_names("Timbre"), vec!["Timbre"]);
    }

    #[test]
    fn meta_split_keeps_slash_names() {
        assert_eq!(meta_artist_names("AC/DC"), vec!["AC/DC"]);
        assert_eq!(meta_artist_names("Tom / Jerry"), vec!["Tom", "Jerry"]);
    }

    #[test]
    fn meta_junk_filtered() {
        assert!(meta_artist_names("muzok.net").is_empty());
        assert!(meta_artist_names("k-nelarecords.blogspot.com").is_empty());
        assert!(meta_artist_names("Various Artists").is_empty());
        assert_eq!(meta_artist_names("Drake, www.promo.ru"), vec!["Drake"]);
    }

    #[test]
    fn meta_unescapes_before_split() {
        assert_eq!(
            meta_artist_names(r"Литвиненко, FLUGY"),
            vec!["Литвиненко", "FLUGY"]
        );
    }

    #[test]
    fn similarity_ladder() {
        assert!((name_similarity("Steve Vai", "SteveVai") - 0.95).abs() < 1e-6);
        assert!(name_similarity("Time Travel (TT)", "Time Travel") >= 0.85);
        assert!(same_artist("GLAM GO GANG!", "Glam Go"));
        assert_eq!(name_similarity("Drake", "Psychosis"), 0.0);
    }

    #[test]
    fn similarity_rejects_infix_containment() {
        assert!(!same_artist("Mark", "Markul"));
        assert!(!same_artist("Иван", "Иванушки International"));
        assert!(!same_artist("Луна", "Лунатик"));
        assert!(same_artist("ultimathule (RUS)", "ultimathule"));
        assert!(same_artist("SODA LUV", "soda luv"));
    }

    #[test]
    fn fold_dollar_as_s() {
        assert!(same_artist("A$AP Rocky", "ASAP Rocky"));
        assert!(same_artist("Ke$ha", "Kesha"));
        assert!(same_artist("1.Kla$", "1.Klas"));
    }

    #[test]
    fn meta_split_matches_title_splitters() {
        assert_eq!(meta_artist_names("Ноггано х Гуф"), vec!["Ноггано", "Гуф"]);
        assert_eq!(meta_artist_names("SALUKI + 104"), vec!["SALUKI", "104"]);
    }

    #[test]
    fn ampersand_folds_to_and() {
        assert!(same_artist(
            "Harold Melvin & The Bluenotes",
            "Harold Melvin And The Bluenotes"
        ));
    }

    #[test]
    fn compare_verdicts() {
        assert_eq!(
            compare_with_meta(["hateclub"], "hateclub"),
            Some(RawMetaMatch::Match)
        );
        assert_eq!(
            compare_with_meta(["ᴍᴏɴᴀʀᴄʜ"], "Monarch"),
            Some(RawMetaMatch::Match)
        );
        assert_eq!(
            compare_with_meta(["Psychosis"], "Psychosis, killaheelz"),
            Some(RawMetaMatch::Partial)
        );
        assert_eq!(
            compare_with_meta(["ᴍᴏɴᴀʀᴄʜ"], "Monarch, johnertekker"),
            Some(RawMetaMatch::Partial)
        );
        assert_eq!(
            compare_with_meta(["Psychosis", "killaheelz"], "Psychosis, killaheelz"),
            Some(RawMetaMatch::Match)
        );
        assert_eq!(
            compare_with_meta(["Cyalm"], "Akio Ohmori, Ritsuo Kamimura"),
            Some(RawMetaMatch::Mismatch)
        );
        assert_eq!(
            compare_with_meta([], "Timbre"),
            Some(RawMetaMatch::Mismatch)
        );
        assert_eq!(compare_with_meta(["GONE.Fludd"], "muzok.net"), None);
        assert_eq!(compare_with_meta(["X"], ""), None);
    }
}
