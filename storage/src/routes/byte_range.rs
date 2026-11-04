#[derive(Debug, PartialEq)]
pub enum ByteRange {
    From { start: u64, end: Option<u64> },
    Suffix(u64),
}

impl ByteRange {
    pub fn parse(value: &str) -> Option<Self> {
        let spec = value.trim().strip_prefix("bytes=")?;
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
    use super::ByteRange;

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
