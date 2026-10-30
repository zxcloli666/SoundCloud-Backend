mod jobs;
mod matching;
mod observe;
mod sync_queue;

use std::fmt::Debug;

use serde::Serialize;
use serde::de::DeserializeOwned;

pub use jobs::{
    AppTokenReject, ColdRefreshClear, Enqueued, EnqueuedJob, JobEnqueue, LyricsWake,
    LyricsWakeState, ObserveClaim,
};
pub use matching::{AlbumSlot, MatchAlbumTrack, MatchGeniusSong, MatchScAccount, MatchWantedLink};
pub use observe::{
    Mode, ObservePlaylists, ObserveProfile, ObserveTracks, ObserveUsers, ObservedPlaylist,
    ObservedTrack, ObservedUser,
};
pub use sync_queue::{ConnectionDropOrphan, SyncQueuePurge, SyncQueueRevive};

pub trait Kind: Serialize + Debug + Sync {
    const NAME: &'static str;
    type Output: DeserializeOwned + Send + Unpin + 'static;
}
