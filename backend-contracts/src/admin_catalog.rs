use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const ADMIN_CATALOG_SUBJECT: &str = "admin.catalog.v1";
pub const ADMIN_CATALOG_QUEUE: &str = "jobs-admin-catalog-v1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminCatalogRequest {
    pub request_id: Uuid,
    pub command: AdminCommand,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminCatalogReply {
    pub status: u16,
    pub body: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", content = "args", rename_all = "snake_case")]
pub enum AdminCommand {
    ArtistCreate(ArtistCreate),
    ArtistUpdate {
        artist_id: Uuid,
        body: ArtistUpdate,
    },
    ArtistMerge {
        src_id: Uuid,
        dst_id: Uuid,
    },
    ArtistRetryCrawl {
        artist_id: Uuid,
    },
    ScAccountUpsert {
        artist_id: Uuid,
        body: ScAccountUpsert,
    },
    ScAccountDelete {
        artist_id: Uuid,
        sc_user_id: String,
    },
    ScAccountDetachTracks {
        artist_id: Uuid,
        sc_user_id: String,
        body: DetachAccountTracks,
    },
    TrackSetPrimaryArtist {
        track_id: Uuid,
        body: SetPrimaryArtist,
    },
    TrackSetAlbum {
        track_id: Uuid,
        body: SetAlbum,
    },
    TrackAddCredit {
        track_id: Uuid,
        body: AddCredit,
    },
    TrackRemoveCredit {
        track_id: Uuid,
        artist_id: Uuid,
        role: String,
    },
    TrackDetachArtist {
        track_id: Uuid,
        body: DetachArtist,
    },
    TrackUnblockArtist {
        track_id: Uuid,
        artist_id: Uuid,
    },
    CreditAccept(CreditRef),
    CreditReject(CreditRef),
    WantedLink {
        wanted_id: Uuid,
        body: WantedLink,
    },
    WantedSetStatus {
        wanted_id: Uuid,
        body: WantedStatus,
    },
    LegacyPlaylistAbandon {
        archive_id: Uuid,
    },
    EnrichRetry(EnrichRetry),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtistCreate {
    pub name: String,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub sc_user_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtistUpdate {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub sc_user_id: Option<String>,
    #[serde(default)]
    pub confidence: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScAccountUpsert {
    pub sc_user_id: String,
    #[serde(default = "main_role")]
    pub role: String,
    #[serde(default)]
    pub notes: Option<String>,
}

fn main_role() -> String {
    "main".to_owned()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetachAccountTracks {
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SetPrimaryArtist {
    pub artist_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SetAlbum {
    #[serde(default)]
    pub album_id: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AddCredit {
    pub artist_id: Uuid,
    #[serde(default = "featured_role")]
    pub role: String,
    #[serde(default)]
    pub position: Option<i16>,
}

pub fn featured_role() -> String {
    "featured".to_owned()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DetachArtist {
    pub artist_id: Uuid,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreditRef {
    pub track_id: Uuid,
    pub artist_id: Uuid,
    pub role: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WantedLink {
    pub sc_track_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WantedStatus {
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnrichRetry {
    #[serde(default)]
    pub sc_track_id: Option<String>,
    #[serde(default)]
    pub all_failed: bool,
}
