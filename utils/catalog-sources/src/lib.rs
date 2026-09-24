mod error;
mod genius;
mod http;
mod lyrics;
mod musicbrainz;
mod throttle;

pub use error::{SourceError, SourceResult};
pub use genius::{
    GeniusAlbumRef, GeniusAlbumTrack, GeniusArtistDetails, GeniusArtistRef, GeniusCandidate,
    GeniusCfg, GeniusService, GeniusSongMeta,
};
pub use http::ExternalFetcher;
pub use lyrics::{
    LyricsCandidate, LyricsFailure, LyricsHints, LyricsLookupOutcome, LyricsSources,
    strip_lrc_timestamps,
};
pub use musicbrainz::{
    MbArtist, MbArtistDetails, MbArtistUrl, MbClient, MbRecording, MbRecordingBrief, MbRelease,
    MbReleaseBrief,
};
pub use throttle::Throttle;
