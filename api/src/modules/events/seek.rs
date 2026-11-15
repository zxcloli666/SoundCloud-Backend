use crate::error::{AppError, AppResult};

pub const SEEK_EVENT: &str = "seek";
const MAX_POSITION_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeekSource {
    Player,
    Moment,
}

impl SeekSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Player => "player",
            Self::Moment => "moment",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seek {
    pub position_ms: i32,
    pub from_ms: i32,
    pub source: SeekSource,
}

impl Seek {
    pub fn parse(
        position_ms: Option<i64>,
        from_ms: Option<i64>,
        source: Option<&str>,
    ) -> AppResult<Self> {
        let source = match source {
            Some("player") => SeekSource::Player,
            Some("moment") => SeekSource::Moment,
            _ => {
                return Err(AppError::bad_request(
                    "source must be \"player\" or \"moment\"",
                ));
            }
        };
        Ok(Self {
            position_ms: milliseconds(position_ms, "positionMs")?,
            from_ms: milliseconds(from_ms, "fromMs")?,
            source,
        })
    }
}

fn milliseconds(value: Option<i64>, name: &str) -> AppResult<i32> {
    value
        .filter(|ms| (0..=MAX_POSITION_MS).contains(ms))
        .and_then(|ms| i32::try_from(ms).ok())
        .ok_or_else(|| {
            AppError::bad_request(format!(
                "{name} must be whole milliseconds between 0 and {MAX_POSITION_MS}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_seek_parses() {
        assert_eq!(
            Seek::parse(Some(61_000), Some(12_500), Some("moment")).ok(),
            Some(Seek {
                position_ms: 61_000,
                from_ms: 12_500,
                source: SeekSource::Moment
            })
        );
    }

    #[test]
    fn missing_negative_or_huge_positions_are_refused() {
        assert!(Seek::parse(None, Some(0), Some("player")).is_err());
        assert!(Seek::parse(Some(0), None, Some("player")).is_err());
        assert!(Seek::parse(Some(-1), Some(0), Some("player")).is_err());
        assert!(Seek::parse(Some(0), Some(MAX_POSITION_MS + 1), Some("player")).is_err());
    }

    #[test]
    fn an_unknown_or_missing_source_is_refused() {
        assert!(Seek::parse(Some(0), Some(0), Some("scrub")).is_err());
        assert!(Seek::parse(Some(0), Some(0), None).is_err());
    }
}
