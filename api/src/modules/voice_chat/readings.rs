use llm_ladder::Role;

use super::answer::{self, Piece};
use super::prompt::EXAMPLE_READINGS;
use super::request::Turn;

const MOST_KNOWN: usize = 64;

#[derive(Default)]
pub struct Readings {
    known: Vec<Reading>,
    asked_in: Vec<Script>,
    held: Vec<char>,
    before: Option<char>,
}

struct Reading {
    written: Vec<char>,
    spoken: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Script {
    Latin,
    Cyrillic,
    Hangul,
    Other,
}

enum Fit {
    None,
    Open,
    Whole { known: usize, len: usize },
}

impl Readings {
    pub fn of(turn: &Turn) -> Self {
        let mut readings = Self::asked(&turn.text);
        for (written, spoken) in EXAMPLE_READINGS {
            readings.learn(written, spoken);
        }
        let replies = turn.history.iter();
        for said in replies.filter(|said| said.role == Role::Assistant) {
            readings.hear(&said.content);
        }
        readings
    }

    fn asked(text: &str) -> Self {
        Self {
            asked_in: scripts(text),
            ..Self::default()
        }
    }

    fn hear(&mut self, said: &str) {
        for (written, spoken) in answer::names(said) {
            self.learn(&written, &spoken);
        }
    }

    pub fn pass(&mut self, piece: Piece) -> Vec<Piece> {
        let Piece::Say(text) = piece else {
            let mut pieces = self.end();
            if let Piece::Name { written, spoken } = &piece {
                self.learn(written, spoken);
                self.before = written.chars().last();
            }
            pieces.push(piece);
            return pieces;
        };
        self.read(&text, false)
    }

    pub fn end(&mut self) -> Vec<Piece> {
        self.read("", true)
    }

    fn learn(&mut self, written: &str, spoken: &str) {
        let written: Vec<char> = written.chars().collect();
        self.known
            .retain(|known| !same_name(&known.written, &written));
        if !self.reads(spoken) {
            return;
        }
        self.known.push(Reading {
            written,
            spoken: spoken.to_owned(),
        });
        if self.known.len() > MOST_KNOWN {
            self.known.remove(0);
        }
    }

    fn reads(&self, spoken: &str) -> bool {
        spoken
            .chars()
            .filter_map(script)
            .any(|script| self.asked_in.contains(&script))
    }

    fn read(&mut self, text: &str, ended: bool) -> Vec<Piece> {
        let mut symbols = std::mem::take(&mut self.held);
        symbols.extend(text.chars());
        let mut pieces = Vec::new();
        let mut plain = String::new();
        let mut at = 0;
        while at < symbols.len() {
            match self.fit(&symbols[at..], ended) {
                Fit::Open => break,
                Fit::None => {
                    plain.push(symbols[at]);
                    self.before = Some(symbols[at]);
                    at += 1;
                }
                Fit::Whole { known, len } => {
                    if !plain.is_empty() {
                        pieces.push(Piece::Say(std::mem::take(&mut plain)));
                    }
                    pieces.push(Piece::Name {
                        written: symbols[at..at + len].iter().collect(),
                        spoken: self.known[known].spoken.clone(),
                    });
                    self.before = Some(symbols[at + len - 1]);
                    at += len;
                }
            }
        }
        self.held = symbols.split_off(at);
        if !plain.is_empty() {
            pieces.push(Piece::Say(plain));
        }
        pieces
    }

    fn fit(&self, rest: &[char], ended: bool) -> Fit {
        if self.before.is_some_and(char::is_alphanumeric) {
            return Fit::None;
        }
        let mut longest = Fit::None;
        for (known, reading) in self.known.iter().enumerate() {
            let len = reading.written.len();
            let shared = len.min(rest.len());
            if !alike(&reading.written[..shared], &rest[..shared]) {
                continue;
            }
            match rest.get(len) {
                Some(next) if next.is_alphanumeric() => {}
                None if !ended => return Fit::Open,
                None if shared < len => {}
                _ if matches!(longest, Fit::Whole { len: found, .. } if found >= len) => {}
                _ => longest = Fit::Whole { known, len },
            }
        }
        longest
    }
}

fn scripts(text: &str) -> Vec<Script> {
    let mut found = Vec::new();
    for script in text.chars().filter_map(script) {
        if !found.contains(&script) {
            found.push(script);
        }
    }
    found
}

fn script(symbol: char) -> Option<Script> {
    let script = match symbol {
        'a'..='z' | 'A'..='Z' | '\u{c0}'..='\u{24f}' => Script::Latin,
        '\u{400}'..='\u{52f}' => Script::Cyrillic,
        '\u{1100}'..='\u{11ff}' | '\u{3130}'..='\u{318f}' | '\u{ac00}'..='\u{d7af}' => {
            Script::Hangul
        }
        _ => Script::Other,
    };
    symbol.is_alphabetic().then_some(script)
}

fn same_name(known: &[char], written: &[char]) -> bool {
    known.len() == written.len() && alike(known, written)
}

fn alike(known: &[char], heard: &[char]) -> bool {
    known
        .iter()
        .zip(heard)
        .all(|(known, heard)| known == heard || known.to_lowercase().eq(heard.to_lowercase()))
}

#[cfg(test)]
#[path = "readings_tests.rs"]
mod tests;
