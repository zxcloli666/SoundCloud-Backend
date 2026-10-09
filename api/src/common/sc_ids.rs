pub use catalog_ingest::{
    EntityKind, EntityRef, extract_sc_id, normalize_sc_track_id, user_id_variants, user_urn,
};

use crate::error::{AppError, AppResult};

pub fn require_ref(kind: EntityKind, input: &str) -> AppResult<EntityRef> {
    EntityRef::parse(kind, input).ok_or_else(|| {
        AppError::bad_request(match kind {
            EntityKind::Track => "invalid track identifier",
            EntityKind::Playlist => "invalid playlist identifier",
            EntityKind::User => "invalid user identifier",
        })
    })
}
