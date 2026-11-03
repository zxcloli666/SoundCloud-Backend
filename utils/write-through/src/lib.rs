mod kinds;
#[cfg(feature = "sqlx")]
mod sqlx_pg;
#[cfg(feature = "tokio-postgres")]
pub mod tokio_pg;

pub use kinds::{
    AlbumSlot, AppTokenReject, ColdRefreshClear, CommentPost, ConnectionDropOrphan,
    ConnectionLogin, Enqueued, EnqueuedJob, JobEnqueue, Kind, LyricsWake, LyricsWakeState,
    MatchAlbumTrack, MatchGeniusSong, MatchScAccount, MatchWantedLink, MembershipOp,
    MembershipOperation, MembershipOutcome, Mode, OauthAppDelete, ObserveClaim, ObservePlaylists,
    ObserveProfile, ObserveTracks, ObserveUsers, ObservedPlaylist, ObservedTrack, ObservedUser,
    OwnedDelete, OwnedInsert, PlaylistDelete, PlaylistEdit, SyncQueuePurge, SyncQueueRevive,
    TrackDelete, TrackEdit, TrackReleaseHeld, TrackS3State, TrackStorageLost, TrackTooLong,
};
#[cfg(feature = "sqlx")]
pub use sqlx_pg::{call, conflicted};

pub fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::SubsecRound::trunc_subsecs(chrono::Utc::now(), 6)
}

pub const RACE_HINT: &str =
    "scd_wt: retry the transaction, a concurrent transaction inserted this key";

pub trait Raced {
    fn raced(&self) -> bool;
}

pub async fn retry<T, E: Raced, F: Future<Output = Result<T, E>>>(
    mut transaction: impl FnMut() -> F,
) -> Result<T, E> {
    match transaction().await {
        Err(error) if error.raced() => transaction().await,
        result => result,
    }
}
