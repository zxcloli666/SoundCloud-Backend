const THINK_OPEN: &str = "<think>";
const THINK_CLOSE: &str = "</think>";

#[derive(Debug, Default)]
pub struct Document {
    state: State,
}

#[derive(Debug)]
enum State {
    Seeking(String),
    Thinking(String),
    Inside(Nesting),
    Complete,
}

enum Opening {
    Think(usize),
    Object(usize),
    Neither,
}

#[derive(Debug, Default)]
struct Nesting {
    depth: usize,
    in_string: bool,
    escaped: bool,
}

impl Document {
    pub fn push(&mut self, chunk: &str) -> String {
        let mut document = String::new();
        let mut input = chunk.to_owned();
        loop {
            match std::mem::take(&mut self.state) {
                State::Seeking(mut seen) => {
                    seen.push_str(&input);
                    match opening(&seen) {
                        Opening::Think(at) => {
                            self.state = State::Thinking(String::new());
                            input = seen[at + THINK_OPEN.len()..].to_owned();
                        }
                        Opening::Object(at) => {
                            self.state = State::Inside(Nesting::default());
                            input = seen[at..].to_owned();
                        }
                        Opening::Neither => {
                            self.state = State::Seeking(unfinished_tag(&seen, THINK_OPEN));
                            return document;
                        }
                    }
                }
                State::Thinking(mut tail) => {
                    tail.push_str(&input);
                    match tail.find(THINK_CLOSE) {
                        Some(close) => {
                            self.state = State::Seeking(String::new());
                            input = tail[close + THINK_CLOSE.len()..].to_owned();
                        }
                        None => {
                            self.state = State::Thinking(unfinished_tag(&tail, THINK_CLOSE));
                            return document;
                        }
                    }
                }
                State::Inside(mut nesting) => {
                    match nesting.close(&input) {
                        Some(end) => {
                            document.push_str(&input[..end]);
                            self.state = State::Complete;
                        }
                        None => {
                            document.push_str(&input);
                            self.state = State::Inside(nesting);
                        }
                    }
                    return document;
                }
                State::Complete => {
                    self.state = State::Complete;
                    return document;
                }
            }
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(self.state, State::Complete)
    }
}

impl Default for State {
    fn default() -> Self {
        Self::Seeking(String::new())
    }
}

impl Nesting {
    fn close(&mut self, text: &str) -> Option<usize> {
        for (at, symbol) in text.char_indices() {
            if self.in_string {
                match symbol {
                    _ if self.escaped => self.escaped = false,
                    '\\' => self.escaped = true,
                    '"' => self.in_string = false,
                    _ => {}
                }
                continue;
            }
            match symbol {
                '"' => self.in_string = true,
                '{' | '[' => self.depth += 1,
                '}' | ']' => {
                    self.depth = self.depth.saturating_sub(1);
                    if self.depth == 0 {
                        return Some(at + symbol.len_utf8());
                    }
                }
                _ => {}
            }
        }
        None
    }
}

fn opening(seen: &str) -> Opening {
    let object = seen.find('{');
    match seen.find(THINK_OPEN) {
        Some(think) if object.is_none_or(|object| think < object) => Opening::Think(think),
        _ => object.map_or(Opening::Neither, Opening::Object),
    }
}

fn unfinished_tag(text: &str, tag: &str) -> String {
    let kept = (1..tag.len())
        .rev()
        .find(|length| text.ends_with(&tag[..*length]))
        .unwrap_or(0);
    text[text.len() - kept..].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn streamed(pieces: &[&str]) -> (String, bool) {
        let mut document = Document::default();
        let body: String = pieces.iter().map(|piece| document.push(piece)).collect();
        (body, document.is_complete())
    }

    fn by_single_characters(text: &str) -> (String, bool) {
        let pieces: Vec<String> = text.chars().map(String::from).collect();
        let pieces: Vec<&str> = pieces.iter().map(String::as_str).collect();
        streamed(&pieces)
    }

    #[test]
    fn a_bare_object_passes_through_unchanged() {
        let text = r#"{"actions":[],"say":"hi"}"#;
        assert_eq!(streamed(&[text]), (text.to_owned(), true));
        assert_eq!(by_single_characters(text), (text.to_owned(), true));
    }

    #[test]
    fn a_markdown_fence_around_the_object_is_dropped() {
        let text = "```json\n{\n  \"say\": \"hi\"\n}\n```";
        let expected = "{\n  \"say\": \"hi\"\n}".to_owned();
        assert_eq!(streamed(&[text]), (expected.clone(), true));
        assert_eq!(by_single_characters(text), (expected, true));
    }

    #[test]
    fn braces_inside_a_think_block_do_not_start_the_object() {
        let text = "<think>I will answer {\"say\": \"no\"} maybe</think>\n{\"say\":\"yes\"}";
        let expected = "{\"say\":\"yes\"}".to_owned();
        assert_eq!(streamed(&[text]), (expected.clone(), true));
        assert_eq!(by_single_characters(text), (expected.clone(), true));
        assert_eq!(
            streamed(&["<thi", "nk>{", "}</thi", "nk>{\"say\":", "\"yes\"}", " bye"]),
            (expected, true)
        );
    }

    #[test]
    fn text_after_the_closing_brace_is_dropped() {
        let (body, complete) = streamed(&["{\"say\":\"a\"}", " and then {\"say\":\"b\"}"]);
        assert_eq!(body, "{\"say\":\"a\"}");
        assert!(complete);
    }

    #[test]
    fn braces_and_quotes_inside_strings_do_not_close_the_object() {
        let text = r#"{"say":"a } \" ] { b","actions":[{"name":"x","args":[]}]}"#;
        assert_eq!(streamed(&[text]), (text.to_owned(), true));
        assert_eq!(by_single_characters(text), (text.to_owned(), true));
    }

    #[test]
    fn a_cut_answer_is_not_complete() {
        let (body, complete) = streamed(&["{\"say\":\"half"]);
        assert_eq!(body, "{\"say\":\"half");
        assert!(!complete);
        assert_eq!(streamed(&["no object here"]), (String::new(), false));
    }

    #[test]
    fn prose_before_the_object_is_dropped_and_non_ascii_survives() {
        let text = "Конечно! {\"say\":\"Привет, мир\"} Вот.";
        assert_eq!(
            streamed(&[text]),
            ("{\"say\":\"Привет, мир\"}".to_owned(), true)
        );
        assert_eq!(
            by_single_characters(text),
            ("{\"say\":\"Привет, мир\"}".to_owned(), true)
        );
    }

    #[test]
    fn an_unclosed_think_block_yields_nothing() {
        let (body, complete) = streamed(&["<think>still {\"say\":\"thinking\"}"]);
        assert_eq!(body, "");
        assert!(!complete);
    }
}
