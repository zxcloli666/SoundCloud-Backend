use serde::Deserialize;

const MAX_YAML_BYTES: usize = 64 * 1024;
const MAX_LINES: usize = 2000;
const MAX_WORDS_PER_LINE: usize = 200;
const MAX_MS: i64 = 6 * 60 * 60 * 1000;
const END_SLACK_MS: i64 = 200;
const PAUSE_MS: i64 = 1000;

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct Rendered {
    pub(crate) synced_lrc: String,
    pub(crate) synced_words: String,
}

#[derive(Deserialize)]
struct RawFile {
    version: String,
    lines: Vec<RawLine>,
}

#[derive(Deserialize)]
struct RawLine {
    text: String,
    start_ms: i64,
    #[serde(default)]
    end_ms: Option<i64>,
    #[serde(default)]
    words: Vec<RawWord>,
}

#[derive(Deserialize)]
struct RawWord {
    text: String,
    start_ms: i64,
}

struct Line {
    text: String,
    start: i64,
    end: Option<i64>,
    tags: Option<Tags>,
}

struct Tags {
    words: Vec<(usize, i64)>,
    closing: i64,
}

pub(crate) fn render(source: &str) -> Option<Rendered> {
    if source.len() > MAX_YAML_BYTES {
        return None;
    }
    let file: RawFile = serde_norway::from_str(source).ok()?;
    if file.version != "1.0" || file.lines.len() > MAX_LINES {
        return None;
    }
    let lines = checked_lines(file.lines)?;
    Some(Rendered {
        synced_lrc: render_lines(&lines, |line| line.text.clone()),
        synced_words: render_lines(&lines, tagged_text),
    })
}

fn checked_lines(raw: Vec<RawLine>) -> Option<Vec<Line>> {
    let mut lines: Vec<Line> = Vec::new();
    for item in raw {
        let text = item.text.trim();
        if text.is_empty() {
            continue;
        }
        if text.contains(['\n', '\r', '\0']) {
            return None;
        }
        let in_order = lines.last().is_none_or(|last| item.start_ms >= last.start);
        let end_valid = item
            .end_ms
            .is_none_or(|end| (item.start_ms..=MAX_MS).contains(&end));
        if !(0..=MAX_MS).contains(&item.start_ms) || !in_order || !end_valid {
            return None;
        }
        let tags = word_tags(text, item.start_ms, item.end_ms, &item.words);
        lines.push(Line {
            text: text.to_owned(),
            start: item.start_ms,
            end: item.end_ms,
            tags,
        });
    }
    (!lines.is_empty()).then_some(lines)
}

fn word_tags(text: &str, start: i64, end: Option<i64>, words: &[RawWord]) -> Option<Tags> {
    let last = words.last()?.start_ms;
    if words.len() > MAX_WORDS_PER_LINE || words[0].start_ms < start || last > MAX_MS {
        return None;
    }
    if end.is_some_and(|end| last > end + END_SLACK_MS) {
        return None;
    }
    if words
        .windows(2)
        .any(|pair| pair[1].start_ms <= pair[0].start_ms)
    {
        return None;
    }
    if words.iter().any(|word| letters(&word.text).is_empty()) {
        return None;
    }
    let joined: String = words.iter().map(|word| word.text.as_str()).collect();
    if letters(&joined) != letters(text) {
        return None;
    }
    let closing = end.unwrap_or(last).max(last);
    let offsets = word_offsets(text, words)?;
    let mut moment = start;
    let tagged = offsets
        .into_iter()
        .zip(words)
        .map(|(offset, word)| {
            moment = word.start_ms.max(moment).min(closing);
            (offset, moment)
        })
        .collect();
    Some(Tags {
        words: tagged,
        closing,
    })
}

fn word_offsets(text: &str, words: &[RawWord]) -> Option<Vec<usize>> {
    let mut cursor = 0;
    let mut offsets = Vec::with_capacity(words.len());
    for word in words {
        let word = word.text.trim();
        let found = cursor + text[cursor..].find(word)?;
        offsets.push(found);
        cursor = found + word.len();
    }
    Some(offsets)
}

fn letters(text: &str) -> String {
    text.chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn render_lines(lines: &[Line], body: impl Fn(&Line) -> String) -> String {
    let mut rendered: Vec<String> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if let Some(previous) = index.checked_sub(1).map(|index| &lines[index])
            && let Some(end) = previous.end
            && line.start - end >= PAUSE_MS
        {
            rendered.push(timestamp(end));
        }
        rendered.push(format!("{}{}", timestamp(line.start), body(line)));
    }
    rendered.join("\n")
}

fn tagged_text(line: &Line) -> String {
    let Some(tags) = &line.tags else {
        return line.text.clone();
    };
    let mut text = String::with_capacity(line.text.len() + (tags.words.len() + 1) * 11);
    let mut cursor = 0;
    for &(offset, moment) in &tags.words {
        text.push_str(&line.text[cursor..offset]);
        text.push_str(&word_tag(moment));
        cursor = offset;
    }
    text.push_str(&line.text[cursor..]);
    text.push_str(&word_tag(tags.closing));
    text
}

fn timestamp(milliseconds: i64) -> String {
    format!("[{}]", clock(milliseconds))
}

fn word_tag(milliseconds: i64) -> String {
    format!("<{}>", clock(milliseconds))
}

fn clock(milliseconds: i64) -> String {
    let centis = (milliseconds + 5) / 10;
    format!(
        "{:02}:{:02}.{:02}",
        centis / 6000,
        centis % 6000 / 100,
        centis % 100
    )
}

#[cfg(test)]
mod tests {
    use regex::Regex;

    use super::*;

    const FILE: &str = "version: '1.0'
metadata:
  title: Invented
lines:
- text: alpha beta
  start_ms: 12340
  end_ms: 13900
  words:
  - text: 'alpha '
    start_ms: 12340
  - text: beta
    start_ms: 13000
- text: gamma
  start_ms: 15000
  end_ms: 15800
- text: delta, epsilon
  start_ms: 16000
  end_ms: 17000
  words:
  - text: delta
    start_ms: 16000
  - text: epsilon
    start_ms: 16500
plain: ignored
";

    fn strip_tags(words: &str) -> String {
        Regex::new(r"<\d{2,}:\d{2}\.\d{2}>")
            .unwrap()
            .replace_all(words, "")
            .into_owned()
    }

    fn with_lines(lines: &str) -> String {
        format!("version: '1.0'\nlines:\n{lines}")
    }

    #[test]
    fn lines_and_words_come_from_the_lyricsfile() {
        let rendered = render(FILE).unwrap();
        assert_eq!(
            rendered.synced_lrc,
            "[00:12.34]alpha beta\n[00:13.90]\n[00:15.00]gamma\n[00:16.00]delta, epsilon"
        );
        assert_eq!(
            rendered.synced_words,
            "[00:12.34]<00:12.34>alpha <00:13.00>beta<00:13.90>\n\
             [00:13.90]\n\
             [00:15.00]gamma\n\
             [00:16.00]<00:16.00>delta, <00:16.50>epsilon<00:17.00>"
        );
        assert_eq!(strip_tags(&rendered.synced_words), rendered.synced_lrc);
    }

    #[test]
    fn a_pause_of_a_second_gets_an_empty_line_at_the_previous_end() {
        let paused = with_lines(
            "- {text: one, start_ms: 1000, end_ms: 2000}\n- {text: two, start_ms: 3000, end_ms: 3500}\n- {text: three, start_ms: 3900}\n",
        );
        let rendered = render(&paused).unwrap();
        assert_eq!(
            rendered.synced_lrc,
            "[00:01.00]one\n[00:02.00]\n[00:03.00]two\n[00:03.90]three"
        );
        assert_eq!(strip_tags(&rendered.synced_words), rendered.synced_lrc);
    }

    #[test]
    fn a_line_with_words_that_cannot_be_placed_stays_untagged() {
        let source = with_lines(
            "- text: one two\n  start_ms: 1000\n  words:\n  - {text: two, start_ms: 1000}\n  - {text: one, start_ms: 1500}\n",
        );
        let rendered = render(&source).unwrap();
        assert_eq!(rendered.synced_words, "[00:01.00]one two");
        let skipped = with_lines(
            "- text: one two three\n  start_ms: 1000\n  words:\n  - {text: one, start_ms: 1000}\n  - {text: three, start_ms: 1500}\n",
        );
        assert_eq!(
            render(&skipped).unwrap().synced_words,
            "[00:01.00]one two three"
        );
    }

    #[test]
    fn word_starts_must_increase_and_stay_inside_the_line() {
        let equal = with_lines(
            "- text: one two\n  start_ms: 1000\n  end_ms: 2000\n  words:\n  - {text: one, start_ms: 1000}\n  - {text: two, start_ms: 1000}\n",
        );
        assert_eq!(render(&equal).unwrap().synced_words, "[00:01.00]one two");
        let early = with_lines(
            "- text: one two\n  start_ms: 1000\n  end_ms: 2000\n  words:\n  - {text: one, start_ms: 900}\n  - {text: two, start_ms: 1500}\n",
        );
        assert_eq!(render(&early).unwrap().synced_words, "[00:01.00]one two");
        let late = with_lines(
            "- text: one two\n  start_ms: 1000\n  end_ms: 2000\n  words:\n  - {text: one, start_ms: 1500}\n  - {text: two, start_ms: 2300}\n",
        );
        assert_eq!(render(&late).unwrap().synced_words, "[00:01.00]one two");
    }

    #[test]
    fn a_line_without_an_end_closes_at_its_last_word() {
        let source = with_lines(
            "- text: one two\n  start_ms: 1000\n  words:\n  - {text: one, start_ms: 1000}\n  - {text: two, start_ms: 1500}\n",
        );
        assert_eq!(
            render(&source).unwrap().synced_words,
            "[00:01.00]<00:01.00>one <00:01.50>two<00:01.50>"
        );
    }

    #[test]
    fn blank_lines_are_dropped() {
        let source = with_lines("- {text: '  ', start_ms: 500}\n- {text: one, start_ms: 1000}\n");
        assert_eq!(render(&source).unwrap().synced_lrc, "[00:01.00]one");
    }

    #[test]
    fn a_broken_file_is_refused() {
        let refused = [
            "not: [valid".to_owned(),
            "version: '2.0'\nlines:\n- {text: one, start_ms: 1}\n".to_owned(),
            "version: '1.0'\nlines: []\n".to_owned(),
            with_lines("- {text: one, start_ms: -1}\n"),
            with_lines("- {text: one, start_ms: 2000}\n- {text: two, start_ms: 1000}\n"),
            with_lines("- {text: one, start_ms: 2000, end_ms: 1000}\n"),
            with_lines("- {text: one, start_ms: 99999999999}\n"),
            with_lines("- {text: \"one\\ntwo\", start_ms: 1000}\n"),
            with_lines("- {text: one}\n"),
            format!("{FILE}#{}", "x".repeat(MAX_YAML_BYTES)),
        ];
        for source in refused {
            assert_eq!(render(&source), None, "{source}");
        }
    }

    #[test]
    fn too_many_lines_are_refused() {
        let lines = "- {text: one, start_ms: 1000}\n".repeat(MAX_LINES + 1);
        assert_eq!(render(&with_lines(&lines)), None);
    }

    #[test]
    fn long_clocks_keep_their_minutes() {
        assert_eq!(clock(6_000_000 + 5_999), "100:06.00");
        assert_eq!(clock(0), "00:00.00");
        assert_eq!(clock(1_994), "00:01.99");
        assert_eq!(clock(1_995), "00:02.00");
    }
}
