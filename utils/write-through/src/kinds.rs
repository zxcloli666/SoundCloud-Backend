mod matching;
mod observe;

use std::fmt::Debug;

use serde::Serialize;
use serde::de::DeserializeOwned;

pub use matching::{AlbumSlot, MatchAlbumTrack, MatchGeniusSong, MatchScAccount, MatchWantedLink};
pub use observe::{
    Mode, ObservePlaylists, ObserveProfile, ObserveTracks, ObserveUsers, ObservedPlaylist,
    ObservedTrack, ObservedUser,
};

pub trait Kind: Serialize + Debug + Sync {
    const NAME: &'static str;
    type Output: DeserializeOwned + Send + Unpin + 'static;
}
