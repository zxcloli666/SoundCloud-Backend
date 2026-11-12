use serde_json::Value;

const ACTIONS: &str = "actions";
const SAY: &str = "say";
const REPLACEMENT: char = '\u{fffd}';
const UNICODE_DIGITS: usize = 4;

#[derive(Debug, PartialEq)]
pub enum Piece {
    Action(Value),
    Say(String),
}

#[derive(Default)]
pub struct Answer {
    at: At,
}

#[derive(Default)]
enum At {
    #[default]
    Before,
    Member,
    Key(Text, String),
    Colon(String),
    Value(String),
    Say(Text),
    Actions(Elements),
    Other(Scan),
    Closed,
}

#[derive(Default)]
struct Text {
    escape: Escape,
    high: Option<u16>,
}

#[derive(Default)]
enum Escape {
    #[default]
    None,
    Started,
    Unicode(u16, usize),
}

#[derive(Default)]
struct Scan {
    depth: usize,
    in_string: bool,
    escaped: bool,
    bare: bool,
}

enum Scanned {
    Inside,
    Ended,
    EndedBefore,
}

#[derive(Default)]
struct Elements {
    raw: String,
    scan: Option<Scan>,
}

enum Element {
    Inside,
    Whole(Value),
    WholeBefore(Value),
    End,
}

impl Answer {
    pub fn push(&mut self, delta: &str) -> Vec<Piece> {
        let mut pieces = Vec::new();
        let mut spoken = String::new();
        for symbol in delta.chars() {
            let mut again = Some(symbol);
            while let Some(symbol) = again.take() {
                if let Some(action) = self.read(symbol, &mut spoken, &mut again) {
                    if !spoken.is_empty() {
                        pieces.push(Piece::Say(std::mem::take(&mut spoken)));
                    }
                    pieces.push(Piece::Action(action));
                }
            }
        }
        if !spoken.is_empty() {
            pieces.push(Piece::Say(spoken));
        }
        pieces
    }

    pub fn is_closed(&self) -> bool {
        matches!(self.at, At::Closed)
    }

    fn read(
        &mut self,
        symbol: char,
        spoken: &mut String,
        again: &mut Option<char>,
    ) -> Option<Value> {
        let mut action = None;
        let next = match &mut self.at {
            At::Before => (symbol == '{').then_some(At::Member),
            At::Member => match symbol {
                '"' => Some(At::Key(Text::default(), String::new())),
                '}' => Some(At::Closed),
                _ => None,
            },
            At::Key(text, name) => text
                .read(symbol, name)
                .then(|| At::Colon(std::mem::take(name))),
            At::Colon(name) => (symbol == ':').then(|| At::Value(std::mem::take(name))),
            At::Value(_) if symbol.is_whitespace() => None,
            At::Value(name) => match (name.as_str(), symbol) {
                (SAY, '"') => Some(At::Say(Text::default())),
                (ACTIONS, '[') => Some(At::Actions(Elements::default())),
                _ => {
                    *again = Some(symbol);
                    Some(At::Other(Scan::default()))
                }
            },
            At::Say(text) => text.read(symbol, spoken).then_some(At::Member),
            At::Actions(elements) => match elements.read(symbol) {
                Element::Inside => None,
                Element::Whole(whole) => {
                    action = Some(whole);
                    None
                }
                Element::WholeBefore(whole) => {
                    action = Some(whole);
                    *again = Some(symbol);
                    None
                }
                Element::End => Some(At::Member),
            },
            At::Other(scan) => match scan.read(symbol) {
                Scanned::Inside => None,
                Scanned::Ended => Some(At::Member),
                Scanned::EndedBefore => {
                    *again = Some(symbol);
                    Some(At::Member)
                }
            },
            At::Closed => None,
        };
        if let Some(next) = next {
            self.at = next;
        }
        action
    }
}

impl Text {
    fn read(&mut self, symbol: char, out: &mut String) -> bool {
        match std::mem::take(&mut self.escape) {
            Escape::None if symbol == '"' => {
                self.close_pair(out);
                return true;
            }
            Escape::None if symbol == '\\' => self.escape = Escape::Started,
            Escape::None => {
                self.close_pair(out);
                out.push(symbol);
            }
            Escape::Started if symbol == 'u' => self.escape = Escape::Unicode(0, 0),
            Escape::Started => {
                self.close_pair(out);
                out.push(unescaped(symbol));
            }
            Escape::Unicode(unit, digits) => match symbol.to_digit(16) {
                Some(digit) => {
                    let unit = (unit << 4) | digit as u16;
                    match digits + 1 {
                        UNICODE_DIGITS => self.unit(unit, out),
                        digits => self.escape = Escape::Unicode(unit, digits),
                    }
                }
                None => {
                    self.close_pair(out);
                    out.push(REPLACEMENT);
                    return self.read(symbol, out);
                }
            },
        }
        false
    }

    fn unit(&mut self, unit: u16, out: &mut String) {
        match (self.high.take(), unit) {
            (Some(high), 0xDC00..=0xDFFF) => {
                let pair = [high, unit];
                out.extend(char::decode_utf16(pair).map(|symbol| symbol.unwrap_or(REPLACEMENT)));
            }
            (pending, 0xD800..=0xDBFF) => {
                out.extend(pending.map(|_| REPLACEMENT));
                self.high = Some(unit);
            }
            (pending, _) => {
                out.extend(pending.map(|_| REPLACEMENT));
                out.push(char::from_u32(u32::from(unit)).unwrap_or(REPLACEMENT));
            }
        }
    }

    fn close_pair(&mut self, out: &mut String) {
        if self.high.take().is_some() {
            out.push(REPLACEMENT);
        }
    }
}

impl Scan {
    fn read(&mut self, symbol: char) -> Scanned {
        if self.in_string {
            match symbol {
                _ if self.escaped => self.escaped = false,
                '\\' => self.escaped = true,
                '"' => {
                    self.in_string = false;
                    if self.depth == 0 {
                        return Scanned::Ended;
                    }
                }
                _ => {}
            }
            return Scanned::Inside;
        }
        match symbol {
            ',' | '}' | ']' if self.depth == 0 => Scanned::EndedBefore,
            _ if self.bare && symbol.is_whitespace() => Scanned::EndedBefore,
            _ if self.bare => Scanned::Inside,
            '"' => {
                self.in_string = true;
                Scanned::Inside
            }
            '{' | '[' => {
                self.depth += 1;
                Scanned::Inside
            }
            '}' | ']' => {
                self.depth -= 1;
                match self.depth {
                    0 => Scanned::Ended,
                    _ => Scanned::Inside,
                }
            }
            _ => {
                self.bare = self.depth == 0 && !symbol.is_whitespace();
                Scanned::Inside
            }
        }
    }
}

impl Elements {
    fn read(&mut self, symbol: char) -> Element {
        let Some(scan) = &mut self.scan else {
            return match symbol {
                ']' => Element::End,
                ',' | '}' => Element::Inside,
                _ if symbol.is_whitespace() => Element::Inside,
                _ => {
                    self.raw.clear();
                    self.scan = Some(Scan::default());
                    self.read(symbol)
                }
            };
        };
        match scan.read(symbol) {
            Scanned::Inside => {
                self.raw.push(symbol);
                Element::Inside
            }
            Scanned::Ended => {
                self.raw.push(symbol);
                Element::Whole(self.take())
            }
            Scanned::EndedBefore => Element::WholeBefore(self.take()),
        }
    }

    fn take(&mut self) -> Value {
        self.scan = None;
        serde_json::from_str(&self.raw).unwrap_or(Value::Null)
    }
}

fn unescaped(symbol: char) -> char {
    match symbol {
        'n' => '\n',
        't' => '\t',
        'r' => '\r',
        'b' => '\u{8}',
        'f' => '\u{c}',
        other => other,
    }
}

#[cfg(test)]
#[path = "answer_tests.rs"]
mod tests;
