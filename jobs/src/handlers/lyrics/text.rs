#[derive(Debug, Eq, PartialEq)]
pub struct ReferenceText {
    pub text: String,
    pub lines_total: i64,
}

pub fn lyrics_text(plain_text: Option<&str>, synced_lrc: Option<&str>) -> Option<String> {
    let plain_text = plain_text.unwrap_or_default().trim();
    if !plain_text.is_empty() {
        return Some(plain_text.to_owned());
    }

    let synced_lrc = synced_lrc.unwrap_or_default().trim();
    if synced_lrc.is_empty() {
        return None;
    }

    let text = synced_lrc
        .lines()
        .filter_map(strip_timestamp)
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

pub fn embedding_text(
    plain_text: Option<&str>,
    synced_lrc: Option<&str>,
    max_bytes: usize,
) -> Option<String> {
    let text = lyrics_text(plain_text, synced_lrc)?;
    let fitted = whole_lines_within(&text, max_bytes);
    let fitted = if fitted.trim().is_empty() {
        prefix_within(&text, max_bytes).trim_end()
    } else {
        fitted
    };
    (!fitted.trim().is_empty()).then(|| fitted.to_owned())
}

pub fn reference_text(plain_text: &str, max_bytes: usize) -> Option<ReferenceText> {
    let plain_text = plain_text.trim();
    let lines_total = plain_text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    let lines_total = i64::try_from(lines_total).ok()?;
    let text = whole_lines_within(plain_text, max_bytes);
    if lines_total == 0 || text.trim().is_empty() {
        return None;
    }
    Some(ReferenceText {
        text: text.to_owned(),
        lines_total,
    })
}

pub fn wire_language(language: Option<&str>) -> Option<String> {
    let language = language?.trim().to_ascii_lowercase();
    (language.len() == 2 && language.bytes().all(|byte| byte.is_ascii_lowercase()))
        .then_some(language)
}

fn whole_lines_within(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = 0;
    for (offset, _) in text.match_indices('\n') {
        if offset > max_bytes {
            break;
        }
        end = offset;
    }
    text.get(..end).unwrap_or_default().trim_end()
}

fn prefix_within(text: &str, max_bytes: usize) -> &str {
    (0..=max_bytes.min(text.len()))
        .rev()
        .find_map(|end| text.get(..end))
        .unwrap_or_default()
}

fn strip_timestamp(line: &str) -> Option<&str> {
    let mut line = line.trim();
    while let Some(rest) = timestamp_suffix(line) {
        line = rest.trim_start();
    }
    (!line.is_empty()).then_some(line)
}

fn timestamp_suffix(line: &str) -> Option<&str> {
    let close = line.strip_prefix('[')?.find(']')? + 1;
    let timestamp = line.get(1..close)?;
    let (minutes, seconds) = timestamp.split_once(':')?;
    let (seconds, fraction) = seconds.split_once('.')?;
    if minutes.len() != 2
        || seconds.len() != 2
        || !(2..=3).contains(&fraction.len())
        || !minutes.bytes().all(|byte| byte.is_ascii_digit())
        || !seconds.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    line.get(close + 1..)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_has_priority() {
        assert_eq!(
            lyrics_text(Some(" plain "), Some("[00:01.00]synced")),
            Some("plain".to_owned())
        );
    }

    #[test]
    fn timestamps_are_removed_from_synced_lyrics() {
        assert_eq!(
            lyrics_text(None, Some("[00:01.00] first\n[00:02.500][00:03.00]second")),
            Some("first\nsecond".to_owned())
        );
    }

    #[test]
    fn reference_text_that_fits_is_sent_whole() {
        let reference = reference_text("first\n\nsecond\n", 100);

        assert_eq!(
            reference,
            Some(ReferenceText {
                text: "first\n\nsecond".to_owned(),
                lines_total: 2,
            })
        );
    }

    #[test]
    fn reference_text_is_cut_only_between_lines_and_counts_every_line() {
        let text = "aaaa\nбббб\ncccc\ndddd";

        let reference = reference_text(text, 17);

        assert_eq!(
            reference,
            Some(ReferenceText {
                text: "aaaa\nбббб".to_owned(),
                lines_total: 4,
            })
        );
    }

    #[test]
    fn a_line_ending_exactly_at_the_limit_is_kept() {
        let reference = reference_text("aaaa\nbbbb\ncccc", 9);

        assert_eq!(
            reference.map(|reference| reference.text),
            Some("aaaa\nbbbb".to_owned())
        );
    }

    #[test]
    fn a_first_line_longer_than_the_limit_leaves_no_reference() {
        assert_eq!(reference_text("ééééé\nshort", 4), None);
        assert_eq!(reference_text("   \n  ", 100), None);
    }

    #[test]
    fn reference_text_never_exceeds_the_wire_limit() {
        let line = "строка текста песни";
        let text = vec![line; 2_000].join("\n");

        let reference = reference_text(&text, 16_000);

        let reference = reference.map(|reference| (reference.text.len(), reference.lines_total));
        assert!(reference.is_some_and(|(bytes, total)| bytes <= 16_000 && total == 2_000));
    }

    #[test]
    fn embedding_text_drops_timestamps_and_respects_the_byte_limit() {
        assert_eq!(
            embedding_text(
                None,
                Some("[00:01.00]one\n[00:02.00]two\n[00:03.00]three"),
                7
            ),
            Some("one\ntwo".to_owned())
        );
    }

    #[test]
    fn only_two_letter_codes_reach_the_wire() {
        assert_eq!(wire_language(Some(" RU ")), Some("ru".to_owned()));
        assert_eq!(wire_language(Some("fil")), None);
        assert_eq!(wire_language(Some("zh-CN")), None);
        assert_eq!(wire_language(Some("")), None);
        assert_eq!(wire_language(None), None);
    }

    #[test]
    fn embedding_text_cuts_an_oversized_single_line_on_a_character_boundary() {
        let text = embedding_text(Some("ёёёёё"), None, 5);

        assert_eq!(text, Some("ёё".to_owned()));
    }

    #[test]
    fn every_byte_limit_cuts_multibyte_lyrics_to_a_whole_character_prefix() {
        let line = "я🎵字ё";
        for max_bytes in 0..=line.len() + 1 {
            let prefix = prefix_within(line, max_bytes);
            assert!(line.starts_with(prefix));
            assert!(prefix.len() <= max_bytes);
            let next = line
                .get(prefix.len()..)
                .and_then(|rest| rest.chars().next());
            assert!(next.is_none_or(|next| prefix.len() + next.len_utf8() > max_bytes));
        }
    }

    #[test]
    fn brackets_next_to_multibyte_text_are_kept_unless_they_hold_a_timestamp() {
        assert_eq!(strip_timestamp("[00:01.00]ёлка"), Some("ёлка"));
        assert_eq!(strip_timestamp("[ё]ёлка"), Some("[ё]ёлка"));
        assert_eq!(strip_timestamp("[00:01.00"), Some("[00:01.00"));
        assert_eq!(strip_timestamp("[]"), Some("[]"));
        assert_eq!(strip_timestamp("[00:01.00]"), None);
    }
}
