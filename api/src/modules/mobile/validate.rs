use crate::error::AppResult;

use super::refusal::invalid;

const TRACK_PREFIX: &str = "soundcloud:tracks:";
const COMMENT_PREFIX: &str = "soundcloud:comments:";
const VALUE_PREFIX: &str = "sc:interactiontypevalue:";
const MAX_ID_LEN: usize = 20;
const MAX_VALUE_LEN: usize = 64;
const MAX_MOMENTS: usize = 40;
pub(super) const MAX_SECOND: u64 = 24 * 60 * 60;

pub(super) fn track_urn(raw: &str) -> AppResult<String> {
    let id = entity_id(raw, TRACK_PREFIX)
        .ok_or_else(|| invalid("invalid_track_urn", "Track urn is not valid"))?;
    Ok(format!("{TRACK_PREFIX}{id}"))
}

pub(super) fn comment_urn(raw: &str) -> AppResult<String> {
    let id = entity_id(raw, COMMENT_PREFIX)
        .filter(|id| !id.starts_with('0'))
        .ok_or_else(|| invalid("invalid_comment_urn", "Comment urn is not valid"))?;
    Ok(format!("{COMMENT_PREFIX}{id}"))
}

pub(super) fn second(second: u64) -> AppResult<u64> {
    if second > MAX_SECOND {
        return Err(invalid(
            "invalid_reaction_second",
            "Reaction second is out of range",
        ));
    }
    Ok(second)
}

pub(super) fn seconds(raw: &str) -> AppResult<Vec<u64>> {
    let mut parsed = Vec::new();
    for part in raw.split(',') {
        let value = part
            .trim()
            .parse::<u64>()
            .map_err(|_| invalid("invalid_reaction_second", "Seconds are not valid"))?;
        let value = second(value)?;
        if !parsed.contains(&value) {
            parsed.push(value);
        }
    }
    if parsed.len() > MAX_MOMENTS {
        return Err(invalid(
            "invalid_reaction_second",
            "Too many seconds in one read",
        ));
    }
    Ok(parsed)
}

pub(super) fn value(raw: Option<&str>) -> AppResult<Option<String>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let name = raw.strip_prefix(VALUE_PREFIX).unwrap_or_default();
    let clean = !name.is_empty()
        && raw.len() <= MAX_VALUE_LEN
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if !clean {
        return Err(invalid(
            "invalid_reaction_value",
            "Reaction value is not valid",
        ));
    }
    Ok(Some(raw.to_owned()))
}

fn entity_id<'a>(raw: &'a str, prefix: &str) -> Option<&'a str> {
    let id = raw.strip_prefix(prefix).unwrap_or(raw);
    let digits = id.bytes().all(|byte| byte.is_ascii_digit());
    (!id.is_empty() && id.len() <= MAX_ID_LEN && digits).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_track_urn_is_normalized_and_its_moment_parses_back() {
        let track = track_urn("123").unwrap_or_default();
        let moment = sc_mobile::reactions::moment_urn(&track, 45);

        assert_eq!(track, "soundcloud:tracks:123");
        assert_eq!(track_urn("soundcloud:tracks:123").ok(), Some(track.clone()));
        assert_eq!(
            sc_mobile::reactions::parse_moment_urn(&moment),
            Some((track.as_str(), 45))
        );
        for bad in ["", "soundcloud:users:1", "1:2", "-1", &"9".repeat(21)] {
            assert!(track_urn(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn seconds_are_deduplicated_and_bounded() {
        assert_eq!(seconds("5, 7,5").ok(), Some(vec![5, 7]));
        assert!(seconds("").is_err());
        assert!(seconds("5,x").is_err());
        assert!(seconds("86401").is_err());
        let many = (0..=MAX_MOMENTS)
            .map(|second| second.to_string())
            .collect::<Vec<_>>();
        assert!(seconds(&many.join(",")).is_err());
    }

    #[test]
    fn a_comment_urn_needs_a_plain_positive_id() {
        assert_eq!(
            comment_urn("55").ok().as_deref(),
            Some("soundcloud:comments:55")
        );
        assert!(comment_urn("soundcloud:comments:055").is_err());
        assert!(comment_urn("soundcloud:tracks:x").is_err());
    }
}
