use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::{Done, Refusal};

pub async fn abandon_legacy(connection: &mut PgConnection, archive_id: Uuid) -> Done {
    let playlist_urn = sqlx::query_file_scalar!("queries/playlists/legacy_abandon.sql", archive_id)
        .fetch_optional(&mut *connection)
        .await?
        .ok_or_else(|| {
            Refusal::not_found("legacy playlist intent not found or already resolved")
        })?;
    let unblocked = sqlx::query_file!("queries/playlists/legacy_wake.sql", &playlist_urn)
        .execute(&mut *connection)
        .await?
        .rows_affected()
        > 0;
    Ok(json!({
        "archiveId": archive_id,
        "playlistUrn": playlist_urn,
        "playlistUnblocked": unblocked,
    }))
}
