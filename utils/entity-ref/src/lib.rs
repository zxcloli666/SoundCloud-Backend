use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntityKind {
    Track,
    Playlist,
    User,
}

impl EntityKind {
    pub const fn segment(self) -> &'static str {
        match self {
            Self::Track => "tracks",
            Self::Playlist => "playlists",
            Self::User => "users",
        }
    }

    pub fn from_segment(segment: &str) -> Option<Self> {
        match segment {
            "tracks" => Some(Self::Track),
            "playlists" => Some(Self::Playlist),
            "users" => Some(Self::User),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntityRef {
    kind: EntityKind,
    id: u64,
}

const URN_PREFIX: &str = "soundcloud:";

impl EntityRef {
    pub fn new(kind: EntityKind, id: u64) -> Option<Self> {
        (id > 0 && id <= i64::MAX as u64).then_some(Self { kind, id })
    }

    pub fn parse(kind: EntityKind, input: &str) -> Option<Self> {
        let input = input.trim();
        match Self::parse_urn(input) {
            Some(parsed) => (parsed.kind == kind).then_some(parsed),
            None if input.contains(':') => None,
            None => Self::new(kind, parse_digits(input)?),
        }
    }

    pub fn parse_urn(input: &str) -> Option<Self> {
        let rest = input.trim().strip_prefix(URN_PREFIX)?;
        let (segment, id) = rest.split_once(':')?;
        Self::new(EntityKind::from_segment(segment)?, parse_digits(id)?)
    }

    pub fn track(input: &str) -> Option<Self> {
        Self::parse(EntityKind::Track, input)
    }

    pub fn playlist(input: &str) -> Option<Self> {
        Self::parse(EntityKind::Playlist, input)
    }

    pub fn user(input: &str) -> Option<Self> {
        Self::parse(EntityKind::User, input)
    }

    pub const fn kind(self) -> EntityKind {
        self.kind
    }

    pub const fn id(self) -> u64 {
        self.id
    }

    pub fn sc_id(self) -> String {
        self.id.to_string()
    }

    pub fn urn(self) -> String {
        format!("{URN_PREFIX}{}:{}", self.kind.segment(), self.id)
    }

    pub fn storage_name(self) -> String {
        self.urn().replace(':', "_")
    }
}

impl fmt::Display for EntityRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{URN_PREFIX}{}:{}", self.kind.segment(), self.id)
    }
}

pub fn track_urn(input: &str) -> Option<String> {
    EntityRef::track(input).map(EntityRef::urn)
}

pub fn playlist_urn(input: &str) -> Option<String> {
    EntityRef::playlist(input).map(EntityRef::urn)
}

pub fn sc_track_id(input: &str) -> Option<String> {
    EntityRef::track(input).map(EntityRef::sc_id)
}

pub fn is_canonical_urn(kind: EntityKind, input: &str) -> bool {
    EntityRef::parse_urn(input).is_some_and(|parsed| parsed.kind == kind && parsed.urn() == input)
}

fn parse_digits(raw: &str) -> Option<u64> {
    if raw.is_empty()
        || raw.len() > 19
        || raw.starts_with('0')
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    raw.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_ids_and_urns_of_the_right_kind_meet_in_one_canonical_form() {
        for input in [
            "42",
            " 42 ",
            "soundcloud:tracks:42",
            " soundcloud:tracks:42",
        ] {
            assert_eq!(
                track_urn(input).as_deref(),
                Some("soundcloud:tracks:42"),
                "{input}"
            );
            assert_eq!(sc_track_id(input).as_deref(), Some("42"), "{input}");
        }
        assert_eq!(playlist_urn("7").as_deref(), Some("soundcloud:playlists:7"));
        assert_eq!(
            EntityRef::user("soundcloud:users:5").map(EntityRef::urn),
            Some("soundcloud:users:5".to_owned())
        );
    }

    #[test]
    fn a_urn_of_another_kind_or_a_foreign_shape_is_never_a_track() {
        for input in [
            "",
            "0",
            "-4",
            "042",
            "soundcloud:tracks:042",
            "abc",
            "4a",
            "soundcloud:users:5",
            "soundcloud:playlists:5",
            "soundcloud:tracks:",
            "soundcloud:tracks:0",
            "soundcloud:tracks:5:6",
            "soundcloud:sounds:5",
            "spotify:tracks:5",
            "tracks:5",
            ":5",
            "99999999999999999999",
            "9223372036854775808",
        ] {
            assert_eq!(EntityRef::track(input), None, "{input:?}");
        }
        assert!(EntityRef::track("9223372036854775807").is_some());
    }

    #[test]
    fn any_kind_is_read_from_a_urn_but_never_from_bare_digits() {
        let parsed = EntityRef::parse_urn("soundcloud:playlists:9").unwrap();
        assert_eq!(parsed.kind(), EntityKind::Playlist);
        assert_eq!(parsed.id(), 9);
        assert_eq!(parsed.to_string(), "soundcloud:playlists:9");
        assert_eq!(EntityRef::parse_urn("9"), None);
    }

    #[test]
    fn storage_names_are_built_from_the_canonical_urn_only() {
        let track = EntityRef::track("12345").unwrap();
        assert_eq!(track.storage_name(), "soundcloud_tracks_12345");
        assert_eq!(
            EntityRef::track("soundcloud:tracks:12345")
                .unwrap()
                .storage_name(),
            "soundcloud_tracks_12345"
        );
        assert!(is_canonical_urn(
            EntityKind::Track,
            "soundcloud:tracks:12345"
        ));
        assert!(!is_canonical_urn(
            EntityKind::Track,
            "soundcloud:tracks:012345"
        ));
        assert!(!is_canonical_urn(EntityKind::Track, "12345"));
        assert!(!is_canonical_urn(
            EntityKind::Track,
            "soundcloud:users:12345"
        ));
    }
}
