use crate::error::{AppError, AppResult};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct TrackSearchQuery {
    pub q: Option<String>,
    pub ids: Option<String>,
    pub genres: Option<String>,
    pub tags: Option<String>,
    pub user_urn: Option<String>,
    pub access: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct PlaylistSearchQuery {
    pub q: Option<String>,
    pub user_urn: Option<String>,
    pub access: Option<String>,
    pub show_tracks: Option<String>,
}

pub(super) fn validate_access(raw: Option<&str>) -> AppResult<()> {
    let Some(raw) = raw else {
        return Ok(());
    };
    if raw.len() <= 64
        && raw
            .split(',')
            .map(str::trim)
            .collect::<std::collections::BTreeSet<_>>()
            == std::collections::BTreeSet::from(["playable", "preview", "blocked"])
    {
        return Ok(());
    }
    Err(AppError::bad_request(
        "Local search does not support filtering by SoundCloud access; omit access",
    ))
}

pub(super) fn parse_terms(raw: Option<&str>, field: &str) -> AppResult<Option<Vec<String>>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.len() > 2048 {
        return Err(AppError::bad_request(format!("{field} is too long")));
    }
    let mut terms = Vec::new();
    for (index, value) in raw.split(',').enumerate() {
        let value = value.trim();
        if index >= 20 || value.is_empty() || value.chars().count() > 128 {
            return Err(AppError::bad_request(format!(
                "{field} accepts up to 20 nonempty values of at most 128 characters"
            )));
        }
        if !terms.iter().any(|term| term == value) {
            terms.push(value.to_owned());
        }
    }
    Ok(Some(terms))
}

pub(super) fn parse_owner(raw: Option<&str>) -> AppResult<Option<String>> {
    let Some(raw) = raw.filter(|s| !s.trim().is_empty()) else {
        return Ok(None);
    };
    if raw.contains(',') {
        return Err(AppError::bad_request("user_urn accepts one user"));
    }
    Ok(parse_ids(Some(raw), "users")?.and_then(|ids| ids.into_iter().next()))
}

const MAX_IDS: usize = 100;

pub(super) fn parse_ids(raw: Option<&str>, namespace: &str) -> AppResult<Option<Vec<String>>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let prefix = format!("soundcloud:{namespace}:");
    let mut ids = Vec::new();
    for (index, value) in raw.split(',').enumerate() {
        if index >= MAX_IDS {
            return Err(AppError::bad_request("ids accepts at most 100 entities"));
        }
        let value = value.trim();
        let bare = value.strip_prefix(&prefix).unwrap_or(value);
        let id = bare
            .parse::<i64>()
            .ok()
            .filter(|id| *id > 0)
            .filter(|_| bare.bytes().all(|byte| byte.is_ascii_digit()))
            .ok_or_else(|| {
                AppError::bad_request(
                    "ids must contain positive IDs or URNs of the requested entity type",
                )
            })?;
        let canonical = id.to_string();
        if !ids.contains(&canonical) {
            ids.push(canonical);
        }
    }
    Ok(Some(ids))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_specific_filters_are_not_silently_ignored() {
        assert!(validate_access(None).is_ok());
        assert!(validate_access(Some("blocked,playable,preview")).is_ok());
        for invalid in ["", "playable", "all", "playable,preview,blocked,private"] {
            assert!(validate_access(Some(invalid)).is_err());
        }
    }

    #[test]
    fn terms_and_owner_filters_are_bounded_and_typed() {
        assert_eq!(
            parse_terms(Some("dub, drum and bass,dub"), "tags").unwrap(),
            Some(vec!["dub".into(), "drum and bass".into()])
        );
        assert!(parse_terms(Some(&vec!["tag"; 21].join(",")), "tags").is_err());
        assert!(parse_terms(Some(""), "tags").is_err());
        assert!(parse_terms(Some(&"x".repeat(129)), "genres").is_err());
        assert_eq!(
            parse_owner(Some("soundcloud:users:17")).unwrap(),
            Some("17".into())
        );
        assert!(parse_owner(Some("17,18")).is_err());
        assert!(parse_owner(Some("soundcloud:tracks:17")).is_err());
        assert!(parse_ids(Some("soundcloud:users:42"), "tracks").is_err());
    }

    #[test]
    fn ids_are_bounded_normalized_and_deduplicated_without_reordering() {
        assert_eq!(
            parse_ids(Some("02, soundcloud:users:1,2"), "users").unwrap(),
            Some(vec!["2".into(), "1".into()])
        );
        for invalid in [
            "",
            "0",
            "-1",
            "+1",
            "1,",
            "soundcloud:tracks:1",
            "9223372036854775808",
        ] {
            assert!(parse_ids(Some(invalid), "users").is_err());
        }
        let too_many = vec!["1"; 101].join(",");
        assert!(parse_ids(Some(&too_many), "users").is_err());
    }
}
