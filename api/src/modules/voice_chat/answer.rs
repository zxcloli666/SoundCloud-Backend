use serde_json::Value;

const ACTIONS: &str = "actions";
const SAY: &str = "say";
const REPLACEMENT: char = '\u{fffd}';
const UNICODE_DIGITS: usize = 4;
const MARK_OPEN: char = '[';
const MARK_CLOSE: char = ']';
const MARK_SPLIT: char = '|';
const MOST_WRITTEN: usize = 80;
const MOST_SPOKEN: usize = 120;

#[derive(Debug, PartialEq)]
pub enum Piece {
    Action(Value),
    Say(String),
    Name { written: String, spoken: String },
}

#[derive(Default)]
pub struct Answer {
    at: At,
    marks: Marks,
}

#[derive(Default)]
struct Pieces {
    done: Vec<Piece>,
    said: String,
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

#[derive(Default)]
struct Marks {
    at: Marked,
}

#[derive(Default)]
enum Marked {
    #[default]
    Plain,
    Opening,
    Inside(Mark),
}

#[derive(Default)]
struct Mark {
    written: String,
    spoken: Option<String>,
    closing: bool,
}

enum Took {
    Inside,
    Closed,
    Broken(Option<char>),
}

impl Answer {
    pub fn push(&mut self, delta: &str) -> Vec<Piece> {
        let mut pieces = Pieces::default();
        for symbol in delta.chars() {
            let mut again = Some(symbol);
            while let Some(symbol) = again.take() {
                self.read(symbol, &mut pieces, &mut again);
            }
        }
        pieces.into_vec()
    }

    pub fn is_closed(&self) -> bool {
        matches!(self.at, At::Closed)
    }

    fn read(&mut self, symbol: char, pieces: &mut Pieces, again: &mut Option<char>) {
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
            At::Say(text) => {
                let mut decoded = String::new();
                let ended = text.read(symbol, &mut decoded);
                for symbol in decoded.chars() {
                    self.marks.read(symbol, pieces);
                }
                if ended {
                    self.marks.end(pieces);
                }
                ended.then_some(At::Member)
            }
            At::Actions(elements) => match elements.read(symbol) {
                Element::Inside => None,
                Element::Whole(whole) => {
                    pieces.push(Piece::Action(whole));
                    None
                }
                Element::WholeBefore(whole) => {
                    pieces.push(Piece::Action(whole));
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
    }
}

impl Pieces {
    fn say(&mut self, symbol: char) {
        self.said.push(symbol);
    }

    fn say_all(&mut self, text: &str) {
        self.said.push_str(text);
    }

    fn push(&mut self, piece: Piece) {
        self.flush();
        self.done.push(piece);
    }

    fn into_vec(mut self) -> Vec<Piece> {
        self.flush();
        self.done
    }

    fn flush(&mut self) {
        if !self.said.is_empty() {
            self.done.push(Piece::Say(std::mem::take(&mut self.said)));
        }
    }
}

impl Marks {
    fn read(&mut self, symbol: char, pieces: &mut Pieces) {
        match std::mem::take(&mut self.at) {
            Marked::Plain if symbol == MARK_OPEN => self.at = Marked::Opening,
            Marked::Plain => pieces.say(symbol),
            Marked::Opening if symbol == MARK_OPEN => self.at = Marked::Inside(Mark::default()),
            Marked::Opening => {
                pieces.say(MARK_OPEN);
                self.read(symbol, pieces);
            }
            Marked::Inside(mut mark) => match mark.read(symbol) {
                Took::Inside => self.at = Marked::Inside(mark),
                Took::Closed => mark.close(pieces),
                Took::Broken(rest) => {
                    pieces.say_all(&mark.written);
                    if let Some(symbol) = rest {
                        self.read(symbol, pieces);
                    }
                }
            },
        }
    }

    fn end(&mut self, pieces: &mut Pieces) {
        match std::mem::take(&mut self.at) {
            Marked::Plain => {}
            Marked::Opening => pieces.say(MARK_OPEN),
            Marked::Inside(mark) => pieces.say_all(&mark.written),
        }
    }
}

impl Mark {
    fn read(&mut self, symbol: char) -> Took {
        if std::mem::take(&mut self.closing) {
            if symbol == MARK_CLOSE {
                return Took::Closed;
            }
            if !self.keep(MARK_CLOSE) {
                return Took::Broken(Some(symbol));
            }
        }
        match symbol {
            MARK_CLOSE => {
                self.closing = true;
                Took::Inside
            }
            MARK_SPLIT if self.spoken.is_none() => {
                self.spoken = Some(String::new());
                Took::Inside
            }
            '\n' | '\r' => Took::Broken(Some(symbol)),
            _ if self.keep(symbol) => Took::Inside,
            _ => Took::Broken(None),
        }
    }

    fn keep(&mut self, symbol: char) -> bool {
        let (part, most) = match &mut self.spoken {
            Some(spoken) => (spoken, MOST_SPOKEN),
            None => (&mut self.written, MOST_WRITTEN),
        };
        part.push(symbol);
        part.chars().count() <= most
    }

    fn close(self, pieces: &mut Pieces) {
        let written = self.written.trim();
        match self.spoken.as_deref().map(str::trim) {
            None => pieces.say_all(&self.written),
            Some(spoken) if spoken.is_empty() || spoken == written => pieces.say_all(written),
            Some(spoken) if written.is_empty() => pieces.say_all(spoken),
            Some(spoken) => pieces.push(Piece::Name {
                written: written.to_owned(),
                spoken: spoken.to_owned(),
            }),
        }
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
