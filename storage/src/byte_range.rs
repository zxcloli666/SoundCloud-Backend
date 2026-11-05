#[derive(Debug, PartialEq)]
pub enum ByteRange {
    From { start: u64, end: Option<u64> },
    Suffix(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slice {
    pub start: u64,
    pub end: u64,
    pub total: u64,
}

impl Slice {
    pub fn len(&self) -> u64 {
        self.end - self.start + 1
    }

    pub fn content_range(&self) -> String {
        format!("bytes {}-{}/{}", self.start, self.end, self.total)
    }

    pub fn parse(value: &str) -> Option<Self> {
        let spec = strip_unit(value.trim(), "bytes ")?;
        let (span, total) = spec.trim().split_once('/')?;
        let (start, end) = span.split_once('-')?;
        let slice = Self {
            start: parse_number(start.trim())?,
            end: parse_number(end.trim())?,
            total: parse_number(total.trim())?,
        };
        (slice.start <= slice.end && slice.end < slice.total).then_some(slice)
    }
}

pub fn unsatisfied(total: u64) -> String {
    format!("bytes */{total}")
}

fn strip_unit<'a>(value: &'a str, unit: &str) -> Option<&'a str> {
    let head = value.get(..unit.len())?;
    head.eq_ignore_ascii_case(unit)
        .then(|| &value[unit.len()..])
}

impl ByteRange {
    pub fn parse(value: &str) -> Option<Self> {
        let spec = strip_unit(value.trim(), "bytes=")?;
        if spec.contains(',') {
            return None;
        }
        let (first, last) = spec.trim().split_once('-')?;
        let (first, last) = (first.trim(), last.trim());
        if first.is_empty() {
            return Some(Self::Suffix(parse_number(last)?));
        }
        let start = parse_number(first)?;
        if last.is_empty() {
            return Some(Self::From { start, end: None });
        }
        let end = parse_number(last)?;
        if end < start {
            return None;
        }
        Some(Self::From {
            start,
            end: Some(end),
        })
    }

    pub fn header(&self) -> String {
        match *self {
            Self::From {
                start,
                end: Some(end),
            } => format!("bytes={start}-{end}"),
            Self::From { start, end: None } => format!("bytes={start}-"),
            Self::Suffix(length) => format!("bytes=-{length}"),
        }
    }

    pub fn never_satisfiable(&self) -> bool {
        matches!(self, Self::Suffix(0))
    }

    pub fn slice(&self, total: u64) -> Option<Slice> {
        self.resolve(total)
            .map(|(start, end)| Slice { start, end, total })
    }

    pub fn resolve(&self, size: u64) -> Option<(u64, u64)> {
        if size == 0 {
            return None;
        }
        match *self {
            Self::From { start, end } if start < size => {
                Some((start, end.map_or(size - 1, |end| end.min(size - 1))))
            }
            Self::From { .. } => None,
            Self::Suffix(0) => None,
            Self::Suffix(n) => Some((size.saturating_sub(n), size - 1)),
        }
    }
}

fn parse_number(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{ByteRange, Slice, unsatisfied};

    #[test]
    fn parses_the_three_single_range_forms() {
        assert_eq!(
            ByteRange::parse("bytes=0-99"),
            Some(ByteRange::From {
                start: 0,
                end: Some(99)
            })
        );
        assert_eq!(
            ByteRange::parse("bytes=500-"),
            Some(ByteRange::From {
                start: 500,
                end: None
            })
        );
        assert_eq!(ByteRange::parse("bytes=-64"), Some(ByteRange::Suffix(64)));
    }

    #[test]
    fn the_unit_is_case_insensitive() {
        for value in ["Bytes=0-99", "BYTES=0-99", " bytes=0-99 "] {
            assert_eq!(
                ByteRange::parse(value),
                Some(ByteRange::From {
                    start: 0,
                    end: Some(99)
                }),
                "{value}"
            );
        }
        assert_eq!(ByteRange::parse("bYtEs=-5"), Some(ByteRange::Suffix(5)));
        assert_eq!(ByteRange::parse("bytés=0-1"), None);
    }

    #[test]
    fn a_range_is_sent_on_in_the_form_it_came() {
        for value in ["bytes=0-99", "bytes=500-", "bytes=-64"] {
            assert_eq!(ByteRange::parse(value).unwrap().header(), value);
        }
        assert_eq!(ByteRange::parse("Bytes=7-").unwrap().header(), "bytes=7-");
        assert!(ByteRange::parse("bytes=-0").unwrap().never_satisfiable());
        assert!(!ByteRange::parse("bytes=0-0").unwrap().never_satisfiable());
    }

    #[test]
    fn a_content_range_gives_the_slice_and_the_total() {
        let slice = Slice::parse("bytes 100-199/1000").unwrap();
        assert_eq!(
            slice,
            Slice {
                start: 100,
                end: 199,
                total: 1000
            }
        );
        assert_eq!(slice.len(), 100);
        assert_eq!(slice.content_range(), "bytes 100-199/1000");
        assert_eq!(Slice::parse("Bytes 0-0/1").map(|s| s.len()), Some(1));
        for value in [
            "bytes */1000",
            "bytes 5-2/1000",
            "bytes 0-1000/1000",
            "bytes 0-9/*",
            "items 0-9/10",
            "",
        ] {
            assert_eq!(Slice::parse(value), None, "{value}");
        }
        assert_eq!(unsatisfied(1000), "bytes */1000");
        assert_eq!(
            ByteRange::parse("bytes=-100").unwrap().slice(1000),
            Some(Slice {
                start: 900,
                end: 999,
                total: 1000
            })
        );
    }

    #[test]
    fn malformed_and_multi_ranges_are_ignored() {
        for value in [
            "",
            "bytes=",
            "bytes=-",
            "bytes=abc-",
            "bytes=5-2",
            "bytes=0-1,4-5",
            "items=0-1",
            "0-1",
            "bytes=+1-2",
            "bytes=1-2-3",
            "bytes=99999999999999999999-",
        ] {
            assert_eq!(ByteRange::parse(value), None, "{value}");
        }
    }

    #[test]
    fn resolves_against_the_object_size() {
        let range = |value| ByteRange::parse(value).unwrap();
        assert_eq!(range("bytes=0-99").resolve(1000), Some((0, 99)));
        assert_eq!(range("bytes=900-5000").resolve(1000), Some((900, 999)));
        assert_eq!(range("bytes=990-").resolve(1000), Some((990, 999)));
        assert_eq!(range("bytes=-100").resolve(1000), Some((900, 999)));
        assert_eq!(range("bytes=-5000").resolve(1000), Some((0, 999)));
        assert_eq!(range("bytes=999-999").resolve(1000), Some((999, 999)));
    }

    #[test]
    fn unsatisfiable_ranges_resolve_to_nothing() {
        let range = |value| ByteRange::parse(value).unwrap();
        assert_eq!(range("bytes=1000-").resolve(1000), None);
        assert_eq!(range("bytes=1000-1010").resolve(1000), None);
        assert_eq!(range("bytes=-0").resolve(1000), None);
        assert_eq!(range("bytes=0-").resolve(0), None);
        assert_eq!(range("bytes=-5").resolve(0), None);
    }
}
