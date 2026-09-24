use crate::title::normalize_title;
use crate::translit::cyrillic_to_latin;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VersionMarker {
    Acoustic,
    Cover,
    Demo,
    Edit,
    Extended,
    Instrumental,
    Live,
    Mashup,
    Remix,
    Reverb,
    Slowed,
    SpedUp,
    Vip,
}

impl VersionMarker {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Acoustic => "acoustic",
            Self::Cover => "cover",
            Self::Demo => "demo",
            Self::Edit => "edit",
            Self::Extended => "extended",
            Self::Instrumental => "instrumental",
            Self::Live => "live",
            Self::Mashup => "mashup",
            Self::Remix => "remix",
            Self::Reverb => "reverb",
            Self::Slowed => "slowed",
            Self::SpedUp => "sped up",
            Self::Vip => "vip",
        }
    }

    fn parse(phrase: &str) -> Option<Self> {
        let phrase = phrase.trim();
        let marker = match phrase {
            "acoustic" | "acoustic version" => Self::Acoustic,
            "cover" | "cover version" => Self::Cover,
            "demo" | "demo version" => Self::Demo,
            "edit" | "radio edit" | "short edit" => Self::Edit,
            "extended" | "extended mix" | "extended version" => Self::Extended,
            "instrumental" | "instrumental version" => Self::Instrumental,
            "live" | "live version" | "live session" => Self::Live,
            "mashup" => Self::Mashup,
            "reverb" | "reverbed" | "with reverb" => Self::Reverb,
            "slowed" | "slowed down" | "slowed reverb" => Self::Slowed,
            "sped up" | "spedup" | "speed up" | "nightcore" => Self::SpedUp,
            "vip" | "vip mix" => Self::Vip,
            _ => return Self::parse_suffix(phrase),
        };
        Some(marker)
    }

    fn parse_suffix(phrase: &str) -> Option<Self> {
        let last = phrase.rsplit(' ').next()?;
        match last {
            "remix" | "rmx" | "flip" | "bootleg" => Some(Self::Remix),
            "edit" => Some(Self::Edit),
            "mashup" => Some(Self::Mashup),
            "mix" if phrase != "original mix" => Some(Self::Remix),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleForms {
    pub work_key: String,
    pub recording_key: String,
    pub aliases: Vec<String>,
    pub markers: Vec<VersionMarker>,
}

impl TitleForms {
    pub fn work_keys(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.work_key.as_str()).chain(self.aliases.iter().map(String::as_str))
    }

    pub fn is_same_recording(&self, other: &Self) -> bool {
        works_match(self, other) && self.markers == other.markers
    }
}

pub fn works_match(left: &TitleForms, right: &TitleForms) -> bool {
    if left.work_key.is_empty() || right.work_key.is_empty() {
        return false;
    }
    left.work_keys()
        .any(|key| right.work_keys().any(|other| other == key))
}

pub fn title_forms(raw: &str) -> TitleForms {
    let (body, groups) = split_groups(raw);
    let mut markers = Vec::new();
    let mut alias_sources = Vec::new();

    for group in groups {
        match VersionMarker::parse(&normalize_title(&group)) {
            Some(marker) => markers.push(marker),
            None => alias_sources.push(group),
        }
    }

    let (base, trailing) = strip_trailing_markers(&body);
    markers.extend(trailing);
    markers.sort_unstable();
    markers.dedup();

    let work_key = normalize_title(&base);
    let aliases = aliases_for(&base, &alias_sources, &work_key);
    let recording_key = recording_key(&work_key, &markers);

    TitleForms {
        work_key,
        recording_key,
        aliases,
        markers,
    }
}

fn recording_key(work_key: &str, markers: &[VersionMarker]) -> String {
    if markers.is_empty() {
        return work_key.to_owned();
    }
    let mut key = String::with_capacity(work_key.len() + markers.len() * 8);
    key.push_str(work_key);
    for marker in markers {
        key.push('|');
        key.push_str(marker.as_str());
    }
    key
}

fn aliases_for(base: &str, groups: &[String], work_key: &str) -> Vec<String> {
    let mut aliases = Vec::new();
    let mut push = |candidate: String| {
        if candidate.is_empty() || candidate == work_key || aliases.contains(&candidate) {
            return;
        }
        aliases.push(candidate);
    };

    if let Some(latin) = cyrillic_to_latin(base) {
        push(normalize_title(&latin));
    }
    for group in groups {
        if !is_translation_of(base, group) {
            continue;
        }
        push(normalize_title(group));
    }
    aliases
}

fn is_translation_of(base: &str, group: &str) -> bool {
    let group = group.trim();
    if group
        .chars()
        .filter(|character| character.is_alphanumeric())
        .count()
        < 2
    {
        return false;
    }
    if group.split_whitespace().count() > 6 {
        return false;
    }
    has_non_latin_letters(base) != has_non_latin_letters(group)
}

fn has_non_latin_letters(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_alphabetic() && (character as u32) > 0x02AF)
}

fn split_groups(raw: &str) -> (String, Vec<String>) {
    let mut body = String::with_capacity(raw.len());
    let mut groups = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;

    for character in raw.chars() {
        match character {
            '(' | '[' => {
                depth += 1;
                if depth == 1 {
                    current.clear();
                    continue;
                }
            }
            ')' | ']' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    groups.push(current.trim().to_owned());
                    continue;
                }
            }
            _ => {}
        }
        if depth == 0 {
            body.push(character);
        } else {
            current.push(character);
        }
    }
    if depth > 0 && !current.trim().is_empty() {
        groups.push(current.trim().to_owned());
    }
    (body, groups)
}

fn strip_trailing_markers(body: &str) -> (String, Vec<VersionMarker>) {
    let mut base = body.trim().to_owned();
    let mut markers = Vec::new();

    while let Some(position) = base.rfind(['-', '–', '—', '|']) {
        let tail = base[position + base[position..].chars().next().map_or(1, char::len_utf8)..]
            .trim()
            .to_owned();
        let Some(marker) = VersionMarker::parse(&normalize_title(&tail)) else {
            break;
        };
        markers.push(marker);
        base = base[..position].trim().to_owned();
    }

    (base, markers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn markers(raw: &str) -> Vec<&'static str> {
        title_forms(raw)
            .markers
            .into_iter()
            .map(VersionMarker::as_str)
            .collect()
    }

    #[test]
    fn parenthetical_translation_becomes_an_alias_not_a_different_work() {
        let ours = title_forms("холод (cold)");
        let genius_native = title_forms("Холод");
        let genius_translated = title_forms("Cold");
        let genius_transliterated = title_forms("Kholod");

        assert_eq!(ours.work_key, "холод");
        assert!(ours.aliases.contains(&"cold".to_owned()));
        assert!(works_match(&ours, &genius_native));
        assert!(works_match(&ours, &genius_translated));
        assert!(works_match(&ours, &genius_transliterated));
    }

    #[test]
    fn latin_parenthetical_is_not_treated_as_translation() {
        let forms = title_forms("Sunday (Bonus Track)");

        assert_eq!(forms.work_key, "sunday");
        assert!(forms.aliases.is_empty());
    }

    #[test]
    fn version_markers_keep_the_work_but_split_the_recording() {
        let original = title_forms("без шансов");
        let sped_up = title_forms("без шансов (sped up)");

        assert!(works_match(&original, &sped_up));
        assert!(!original.is_same_recording(&sped_up));
        assert_eq!(sped_up.markers, vec![VersionMarker::SpedUp]);
        assert_eq!(sped_up.recording_key, "без шансов|sped up");
    }

    #[test]
    fn remix_credit_is_a_version_marker() {
        assert_eq!(markers("Track (Skrillex Remix)"), vec!["remix"]);
        assert_eq!(markers("Track - Live"), vec!["live"]);
        assert_eq!(markers("Track [Radio Edit]"), vec!["edit"]);
    }

    #[test]
    fn original_mix_is_noise_and_not_a_remix() {
        let forms = title_forms("Track (Original Mix)");

        assert!(forms.markers.is_empty());
        assert_eq!(forms.work_key, "track");
    }

    #[test]
    fn several_markers_are_sorted_and_deduplicated() {
        let forms = title_forms("Track (Slowed) (slowed) [Live]");

        assert_eq!(
            forms.markers,
            vec![VersionMarker::Live, VersionMarker::Slowed]
        );
    }

    #[test]
    fn empty_title_never_matches() {
        assert!(!works_match(&title_forms(""), &title_forms("")));
    }
}
