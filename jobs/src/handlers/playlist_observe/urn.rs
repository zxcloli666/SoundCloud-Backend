use std::fmt;

const PREFIX: &str = "soundcloud:playlists:";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlaylistUrn {
    value: String,
    id: String,
}

#[derive(Debug, thiserror::Error)]
#[error("playlist URN must be canonical soundcloud:playlists:<id>")]
pub struct InvalidPlaylistUrn;

impl PlaylistUrn {
    pub fn parse(value: &str) -> Result<Self, InvalidPlaylistUrn> {
        let id = value.strip_prefix(PREFIX).ok_or(InvalidPlaylistUrn)?;
        let number = id.parse::<u64>().map_err(|_| InvalidPlaylistUrn)?;
        if number == 0 || number.to_string() != id {
            return Err(InvalidPlaylistUrn);
        }
        Ok(Self {
            value: value.to_owned(),
            id: id.to_owned(),
        })
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

impl fmt::Display for PlaylistUrn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_playlist_urn() {
        let urn = PlaylistUrn::parse("soundcloud:playlists:2278082420").unwrap();

        assert_eq!(urn.id(), "2278082420");
        assert_eq!(urn.as_str(), "soundcloud:playlists:2278082420");
    }

    #[test]
    fn rejects_noncanonical_playlist_identifiers() {
        for value in [
            "2278082420",
            "soundcloud:tracks:2278082420",
            "soundcloud:playlists:02278082420",
            "soundcloud:playlists:0",
            "soundcloud:playlists:-1",
            "soundcloud:playlists:1?x=2",
        ] {
            assert!(PlaylistUrn::parse(value).is_err(), "accepted {value}");
        }
    }
}
