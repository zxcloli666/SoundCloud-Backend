use serde::{Deserialize, Serialize};

use crate::{CatalogEntity, CatalogRefreshPayload};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CatalogCollection {
    LikedTracks,
    LikedPlaylists,
    Followings,
    Followers,
    OwnedTracks,
    OwnedPlaylists,
    TrackFavoriters,
    TrackReposters,
    PlaylistReposters,
    TrackComments,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CollectionItem {
    Entity(CatalogEntity),
    Comment,
}

impl CatalogCollection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LikedTracks => "liked-tracks",
            Self::LikedPlaylists => "liked-playlists",
            Self::Followings => "followings",
            Self::Followers => "followers",
            Self::OwnedTracks => "owned-tracks",
            Self::OwnedPlaylists => "owned-playlists",
            Self::TrackFavoriters => "track-favoriters",
            Self::TrackReposters => "track-reposters",
            Self::PlaylistReposters => "playlist-reposters",
            Self::TrackComments => "track-comments",
        }
    }

    pub const fn item(self) -> CollectionItem {
        match self {
            Self::TrackComments => CollectionItem::Comment,
            other => CollectionItem::Entity(other.entity()),
        }
    }

    pub const fn entity(self) -> CatalogEntity {
        match self {
            Self::LikedTracks | Self::OwnedTracks => CatalogEntity::Track,
            Self::LikedPlaylists | Self::OwnedPlaylists => CatalogEntity::Playlist,
            Self::Followings
            | Self::Followers
            | Self::TrackFavoriters
            | Self::TrackReposters
            | Self::PlaylistReposters
            | Self::TrackComments => CatalogEntity::User,
        }
    }

    pub const fn subject(self) -> CatalogEntity {
        match self {
            Self::TrackFavoriters | Self::TrackReposters | Self::TrackComments => {
                CatalogEntity::Track
            }
            Self::PlaylistReposters => CatalogEntity::Playlist,
            _ => CatalogEntity::User,
        }
    }

    pub const fn public_apiv2(self) -> bool {
        !matches!(self, Self::TrackFavoriters)
    }

    pub const fn public_only(self) -> bool {
        matches!(
            self,
            Self::Followers
                | Self::TrackFavoriters
                | Self::TrackReposters
                | Self::PlaylistReposters
                | Self::TrackComments
        )
    }

    pub const fn path_segment(self, apiv2: bool) -> &'static str {
        match self {
            Self::LikedTracks if apiv2 => "track_likes",
            Self::LikedPlaylists if apiv2 => "playlist_likes",
            Self::LikedTracks => "likes/tracks",
            Self::LikedPlaylists => "likes/playlists",
            Self::Followings => "followings",
            Self::Followers => "followers",
            Self::OwnedTracks => "tracks",
            Self::OwnedPlaylists => "playlists",
            Self::TrackFavoriters => "favoriters",
            Self::TrackReposters | Self::PlaylistReposters => "reposters",
            Self::TrackComments => "comments",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogCollectionPayload {
    pub collection: CatalogCollection,
    pub subject_id: String,
    pub owner: bool,
}

impl CatalogCollectionPayload {
    pub fn is_valid(&self) -> bool {
        if self.owner && self.collection.public_only() {
            return false;
        }
        CatalogRefreshPayload {
            entity: self.collection.subject(),
            sc_id: self.subject_id.clone(),
            owner_id: None,
        }
        .is_valid()
    }

    pub const fn scope(&self) -> &'static str {
        if self.owner { "owner" } else { "public" }
    }

    pub fn dedup_key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.collection.as_str(),
            self.subject_id,
            self.scope()
        )
    }

    pub fn path(&self, apiv2: bool) -> String {
        let segment = self.collection.path_segment(apiv2);
        if self.owner {
            return format!("/me/{segment}");
        }
        let root = match self.collection.subject() {
            CatalogEntity::Track => "tracks",
            CatalogEntity::Playlist => "playlists",
            _ => "users",
        };
        format!("/{root}/{}/{segment}", self.subject_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn followers_always_use_the_shared_public_resource() {
        let mut payload = CatalogCollectionPayload {
            collection: CatalogCollection::Followers,
            subject_id: "42".into(),
            owner: false,
        };
        assert!(payload.is_valid());
        assert_eq!(payload.path(false), "/users/42/followers");
        assert_eq!(payload.path(true), "/users/42/followers");
        assert_eq!(payload.dedup_key(), "followers:42:public");
        assert_eq!(payload.collection.entity(), CatalogEntity::User);
        payload.owner = true;
        assert!(!payload.is_valid());
    }

    #[test]
    fn audience_collections_address_the_entity_that_owns_them() {
        let mut payload = CatalogCollectionPayload {
            collection: CatalogCollection::TrackFavoriters,
            subject_id: "42".into(),
            owner: false,
        };
        assert!(payload.is_valid());
        assert_eq!(payload.path(true), "/tracks/42/favoriters");
        assert_eq!(payload.dedup_key(), "track-favoriters:42:public");
        assert_eq!(payload.collection.entity(), CatalogEntity::User);
        assert_eq!(payload.collection.subject(), CatalogEntity::Track);
        payload.collection = CatalogCollection::TrackReposters;
        assert_eq!(payload.path(true), "/tracks/42/reposters");
        payload.collection = CatalogCollection::PlaylistReposters;
        assert_eq!(payload.path(true), "/playlists/42/reposters");
        assert_eq!(payload.collection.subject(), CatalogEntity::Playlist);
        for collection in [
            CatalogCollection::TrackFavoriters,
            CatalogCollection::TrackReposters,
            CatalogCollection::PlaylistReposters,
        ] {
            payload.collection = collection;
            payload.owner = true;
            assert!(!payload.is_valid());
            payload.owner = false;
            assert!(payload.is_valid());
        }
    }

    #[test]
    fn collection_scope_determines_identity_and_transport() {
        let mut payload = CatalogCollectionPayload {
            collection: CatalogCollection::LikedTracks,
            subject_id: "42".into(),
            owner: false,
        };
        assert!(payload.is_valid());
        assert_eq!(payload.path(true), "/users/42/track_likes");
        assert_eq!(payload.path(false), "/users/42/likes/tracks");
        let public = payload.dedup_key();
        payload.owner = true;
        assert_ne!(payload.dedup_key(), public);
        assert_eq!(payload.path(false), "/me/likes/tracks");
        for id in ["0", "01", "-1", "42/tracks", "soundcloud:users:42"] {
            payload.subject_id = id.into();
            assert!(!payload.is_valid());
        }
    }
}
