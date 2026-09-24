pub mod collection;
pub mod entity;
mod read;
pub mod service;

#[cfg(test)]
mod page_tests;

#[cfg(test)]
mod collection_tests;

#[cfg(test)]
mod follower_tests;

#[cfg(test)]
mod audience_tests;

pub use read::{read_audience_page, read_collection_page};
pub use service::{
    AudienceCollection, ColdRefreshService, FOLLOWERS, FOLLOWINGS, LIKED_PLAYLISTS, LIKED_TRACKS,
    OWNED_PLAYLISTS, OWNED_TRACKS, PLAYLIST_REPOSTERS, TRACK_FAVORITERS, TRACK_REPOSTERS,
};
