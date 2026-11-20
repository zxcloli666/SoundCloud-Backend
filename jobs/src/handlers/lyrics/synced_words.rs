const MAX_SYNCED_WORDS_BYTES: usize = 1_600_000;

pub fn accepted_synced_words(
    sc_track_id: &str,
    synced_lrc: &str,
    synced_words: Option<String>,
) -> Option<String> {
    let words = synced_words?;
    match refusal(synced_lrc, &words) {
        None => Some(words),
        Some(reason) => {
            tracing::warn!(track = %sc_track_id, reason, "word timings dropped, line timings kept");
            None
        }
    }
}

fn refusal(synced_lrc: &str, words: &str) -> Option<&'static str> {
    if words.trim().is_empty() {
        return Some("blank");
    }
    if words.len() > MAX_SYNCED_WORDS_BYTES {
        return Some("too large");
    }
    if words.contains('\0') {
        return Some("null byte");
    }
    if strip_word_tags(words) != synced_lrc {
        return Some("lines differ from the synced lyrics");
    }
    None
}

fn strip_word_tags(words: &str) -> String {
    let mut stripped = String::with_capacity(words.len());
    let mut rest = words;
    while let Some(open) = rest.find('<') {
        stripped.push_str(&rest[..open]);
        let tail = &rest[open..];
        match word_tag_len(tail) {
            Some(length) => rest = &tail[length..],
            None => {
                stripped.push('<');
                rest = &tail[1..];
            }
        }
    }
    stripped.push_str(rest);
    stripped
}

fn word_tag_len(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let minutes = bytes[1..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    let tail = &bytes[1 + minutes..];
    let shape = minutes >= 2
        && tail.len() >= 7
        && tail[0] == b':'
        && tail[1..3].iter().all(u8::is_ascii_digit)
        && tail[3] == b'.'
        && tail[4..6].iter().all(u8::is_ascii_digit)
        && tail[6] == b'>';
    shape.then_some(1 + minutes + 7)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LRC: &str = "[00:01.00]one two\n[00:03.00]\n[00:04.00]a < b";
    const WORDS: &str =
        "[00:01.00]<00:01.00>one <00:01.50>two<00:02.00>\n[00:03.00]\n[00:04.00]a < b";

    #[test]
    fn tags_are_stripped_and_other_angle_brackets_stay() {
        assert_eq!(strip_word_tags(WORDS), LRC);
        assert_eq!(
            strip_word_tags("x <1:2> <00:01.5> y"),
            "x <1:2> <00:01.5> y"
        );
    }

    #[test]
    fn words_that_match_the_lines_are_kept() {
        assert_eq!(
            accepted_synced_words("42", LRC, Some(WORDS.to_owned())).as_deref(),
            Some(WORDS)
        );
    }

    #[test]
    fn words_that_differ_blank_or_missing_are_dropped() {
        assert_eq!(accepted_synced_words("42", LRC, None), None);
        assert_eq!(
            accepted_synced_words("42", LRC, Some("  ".to_owned())),
            None
        );
        assert_eq!(
            accepted_synced_words("42", LRC, Some(WORDS.replace("one", "uno"))),
            None
        );
        assert_eq!(
            accepted_synced_words("42", LRC, Some(format!("{WORDS}\0"))),
            None
        );
    }
}
