use catalog_sources::GeniusText;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Verdict {
    Replace,
    Unchanged,
    Different,
}

pub(super) fn verdict(stored: &str, candidates: &[GeniusText]) -> Verdict {
    let old = comparable(without_header(stored));
    let mut unchanged = false;
    for candidate in candidates {
        let new = comparable(&candidate.text);
        if new == old {
            unchanged = true;
        } else if replaces(&old, &new, candidate.linked) {
            return Verdict::Replace;
        }
    }
    if unchanged {
        Verdict::Unchanged
    } else {
        Verdict::Different
    }
}

fn replaces(old: &str, new: &str, linked: bool) -> bool {
    if new.len() <= old.len() {
        return false;
    }
    linked || (!old.is_empty() && new.ends_with(&format!(" {old}")))
}

fn without_header(stored: &str) -> &str {
    let (first, rest) = match stored.split_once('\n') {
        Some(parts) => parts,
        None => (stored, ""),
    };
    if is_header(first) { rest } else { stored }
}

fn is_header(line: &str) -> bool {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && line[digits..].starts_with(" Contributor")
}

fn comparable(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn searched(text: &str) -> GeniusText {
        GeniusText {
            text: text.to_owned(),
            linked: false,
        }
    }

    fn linked(text: &str) -> GeniusText {
        GeniusText {
            text: text.to_owned(),
            linked: true,
        }
    }

    #[test]
    fn a_plural_header_line_is_dropped() {
        let stored = "879 ContributorsTranslationsDeutsch\nline two\nline three";
        let new = searched("line one\nline two\nline three");
        assert_eq!(verdict(stored, &[new]), Verdict::Replace);
    }

    #[test]
    fn a_singular_header_line_is_dropped() {
        let stored = "1 ContributorM1 Lyrics\nline two\nline three";
        let new = searched("line one\nline two\nline three");
        assert_eq!(verdict(stored, &[new]), Verdict::Replace);
    }

    #[test]
    fn a_text_without_a_header_is_kept_whole() {
        let stored = "line one\nline two";
        assert_eq!(
            verdict(stored, &[searched("line one\nline two")]),
            Verdict::Unchanged
        );
        assert_eq!(verdict(stored, &[searched("line two")]), Verdict::Different);
    }

    #[test]
    fn blank_lines_and_whitespace_are_ignored() {
        let stored = "line one\n\n  line   two \n";
        assert_eq!(
            verdict(stored, &[searched("line one\nline two")]),
            Verdict::Unchanged
        );
    }

    #[test]
    fn the_old_text_must_start_at_a_word_boundary() {
        let stored = "ar C";
        assert_eq!(verdict(stored, &[searched("A bar C")]), Verdict::Different);
        assert_eq!(verdict(stored, &[searched("A ar C")]), Verdict::Replace);
    }

    #[test]
    fn an_equal_text_is_unchanged() {
        assert_eq!(
            verdict("same text", &[linked("same text")]),
            Verdict::Unchanged
        );
    }

    #[test]
    fn a_longer_linked_candidate_with_another_tail_replaces() {
        let stored = "old tail";
        assert_eq!(
            verdict(stored, &[linked("the page was edited, new tail")]),
            Verdict::Replace
        );
    }

    #[test]
    fn a_linked_candidate_that_is_not_longer_is_different() {
        assert_eq!(
            verdict("a longer stored text", &[linked("short page")]),
            Verdict::Different
        );
    }

    #[test]
    fn a_searched_candidate_does_not_replace_a_header_only_text() {
        let stored = "12 Contributors";
        assert_eq!(
            verdict(stored, &[searched("whole song text")]),
            Verdict::Different
        );
    }

    #[test]
    fn a_linked_candidate_replaces_a_header_only_text() {
        let stored = "12 Contributors";
        assert_eq!(
            verdict(stored, &[linked("whole song text")]),
            Verdict::Replace
        );
    }

    #[test]
    fn the_first_replacing_candidate_wins() {
        let stored = "tail";
        let candidates = [
            searched("tail"),
            searched("other song"),
            searched("head tail"),
            linked("another head and more tail"),
        ];
        assert_eq!(verdict(stored, &candidates), Verdict::Replace);
    }

    #[test]
    fn an_unchanged_candidate_beats_different_ones_when_none_replaces() {
        let stored = "tail";
        let candidates = [searched("other song"), searched("tail")];
        assert_eq!(verdict(stored, &candidates), Verdict::Unchanged);
    }

    #[test]
    fn no_candidates_is_different() {
        assert_eq!(verdict("tail", &[]), Verdict::Different);
    }
}
