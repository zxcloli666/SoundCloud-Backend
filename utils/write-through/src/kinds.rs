mod jobs;
mod matching;
mod mutations;
mod observe;
mod storage;
mod sync_queue;

use std::fmt::Debug;

use serde::Serialize;
use serde::de::DeserializeOwned;

pub use jobs::{
    AppTokenReject, ColdRefreshClear, Enqueued, EnqueuedJob, JobEnqueue, LyricsWake,
    LyricsWakeState, ObserveClaim,
};
pub use matching::{AlbumSlot, MatchAlbumTrack, MatchGeniusSong, MatchScAccount, MatchWantedLink};
pub use mutations::{
    CommentPost, MembershipOp, MembershipOperation, MembershipOutcome, OwnedDelete, OwnedInsert,
    PlaylistDelete, PlaylistEdit, TrackDelete, TrackEdit,
};
pub use observe::{
    Mode, ObservePlaylists, ObserveProfile, ObserveTracks, ObserveUsers, ObservedPlaylist,
    ObservedTrack, ObservedUser,
};
pub use storage::TrackStorageLost;
pub use sync_queue::{ConnectionDropOrphan, ConnectionLogin, SyncQueuePurge, SyncQueueRevive};

pub trait Kind: Serialize + Debug + Sync {
    const NAME: &'static str;
    type Output: DeserializeOwned + Send + Unpin + 'static;
}
