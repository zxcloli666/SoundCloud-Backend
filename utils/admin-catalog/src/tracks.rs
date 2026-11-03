use backend_contracts::admin_catalog::{
    AddCredit, DetachAccountTracks, DetachArtist, SetAlbum, SetPrimaryArtist,
};
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::{Done, Fail, Refusal};

const CREDIT_ROLES: [&str; 4] = ["primary", "featured", "remixer", "producer"];

fn canonical_role(role: &str) -> String {
    let role = role.trim().to_lowercase();
    if role == "feature" {
        "featured".to_string()
    } else {
        role
    }
}

async fn require_artist(connection: &mut PgConnection, artist_id: Uuid) -> Result<(), Fail> {
    let exists = sqlx::query_file_scalar!("queries/artists/exists.sql", artist_id)
        .fetch_one(&mut *connection)
        .await?;
    if !exists {
        return Err(Refusal::bad_request("artist not found").into());
    }
    Ok(())
}

async fn require_track(connection: &mut PgConnection, track_id: Uuid) -> Result<(), Fail> {
    let exists = sqlx::query_file_scalar!("queries/tracks/exists.sql", track_id)
        .fetch_one(&mut *connection)
        .await?;
    if !exists {
        return Err(Refusal::not_found("track not found").into());
    }
    Ok(())
}

pub async fn set_primary_artist(
    connection: &mut PgConnection,
    track_id: Uuid,
    body: &SetPrimaryArtist,
) -> Done {
    require_artist(connection, body.artist_id).await?;
    require_track(connection, track_id).await?;
    sqlx::query_file!(
        "queries/tracks/block_delete_pair.sql",
        track_id,
        body.artist_id
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query_file!("queries/tracks/credit_delete_primary.sql", track_id)
        .execute(&mut *connection)
        .await?;
    sqlx::query_file!(
        "queries/tracks/credit_insert_primary.sql",
        track_id,
        body.artist_id
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query_file!(
        "queries/tracks/set_primary_artist.sql",
        body.artist_id,
        track_id
    )
    .execute(&mut *connection)
    .await?;
    Ok(json!({ "ok": true }))
}

pub async fn set_album(connection: &mut PgConnection, track_id: Uuid, body: &SetAlbum) -> Done {
    if let Some(album_id) = body.album_id {
        let exists = sqlx::query_file_scalar!("queries/tracks/album_exists.sql", album_id)
            .fetch_one(&mut *connection)
            .await?;
        if !exists {
            return Err(Refusal::bad_request("album not found").into());
        }
    }
    let updated = sqlx::query_file!("queries/tracks/set_album.sql", body.album_id, track_id)
        .execute(&mut *connection)
        .await?;
    if updated.rows_affected() == 0 {
        return Err(Refusal::not_found("track not found").into());
    }
    sqlx::query_file!("queries/tracks/album_tracks_delete_by_track.sql", track_id)
        .execute(&mut *connection)
        .await?;
    if let Some(album_id) = body.album_id {
        sqlx::query_file!("queries/tracks/album_tracks_insert.sql", album_id, track_id)
            .execute(&mut *connection)
            .await?;
    }
    Ok(json!({ "ok": true, "album_id": body.album_id }))
}

pub async fn add_credit(connection: &mut PgConnection, track_id: Uuid, body: &AddCredit) -> Done {
    let role = canonical_role(&body.role);
    if !CREDIT_ROLES.contains(&role.as_str()) {
        return Err(Refusal::bad_request(
            "role must be one of: primary, featured, remixer, producer",
        )
        .into());
    }
    require_artist(connection, body.artist_id).await?;
    require_track(connection, track_id).await?;
    sqlx::query_file!(
        "queries/tracks/block_delete_pair.sql",
        track_id,
        body.artist_id
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query_file!(
        "queries/tracks/credit_upsert.sql",
        track_id,
        body.artist_id,
        &role,
        body.position.map(i32::from)
    )
    .execute(&mut *connection)
    .await?;
    if role == "primary" {
        sqlx::query_file!(
            "queries/tracks/credit_delete_other_primary.sql",
            track_id,
            body.artist_id
        )
        .execute(&mut *connection)
        .await?;
        sqlx::query_file!(
            "queries/tracks/set_primary_artist.sql",
            body.artist_id,
            track_id
        )
        .execute(&mut *connection)
        .await?;
    }
    Ok(json!({ "ok": true, "role": role }))
}

pub async fn remove_credit(
    connection: &mut PgConnection,
    track_id: Uuid,
    artist_id: Uuid,
    role: &str,
) -> Done {
    let role = canonical_role(role);
    let removed = sqlx::query_file!(
        "queries/tracks/credit_delete.sql",
        track_id,
        artist_id,
        &role
    )
    .execute(&mut *connection)
    .await?
    .rows_affected();
    if role == "primary" {
        sqlx::query_file!(
            "queries/tracks/clear_primary_if_match.sql",
            track_id,
            artist_id
        )
        .execute(&mut *connection)
        .await?;
    }
    Ok(json!({ "ok": true, "removed": removed }))
}

pub async fn detach_artist(
    connection: &mut PgConnection,
    track_id: Uuid,
    body: &DetachArtist,
) -> Done {
    require_track(connection, track_id).await?;
    sqlx::query_file!(
        "queries/tracks/block_upsert.sql",
        track_id,
        body.artist_id,
        body.note.as_deref()
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query_file!(
        "queries/tracks/credit_delete_pair.sql",
        track_id,
        body.artist_id
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query_file!(
        "queries/tracks/clear_primary_if_match.sql",
        track_id,
        body.artist_id
    )
    .execute(&mut *connection)
    .await?;
    Ok(json!({ "ok": true }))
}

pub async fn unblock_artist(
    connection: &mut PgConnection,
    track_id: Uuid,
    artist_id: Uuid,
) -> Done {
    let removed = sqlx::query_file!("queries/tracks/block_delete_pair.sql", track_id, artist_id)
        .execute(&mut *connection)
        .await?
        .rows_affected();
    Ok(json!({ "ok": true, "removed": removed }))
}

pub async fn detach_account_tracks(
    connection: &mut PgConnection,
    artist_id: Uuid,
    sc_user_id: &str,
    body: &DetachAccountTracks,
) -> Done {
    let detached = sqlx::query_file_scalar!(
        "queries/tracks/sc_account_detach_count.sql",
        artist_id,
        sc_user_id
    )
    .fetch_one(&mut *connection)
    .await?;
    sqlx::query_file!(
        "queries/tracks/sc_account_detach_block.sql",
        artist_id,
        body.note.as_deref(),
        sc_user_id
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query_file!(
        "queries/tracks/sc_account_detach_delete_credits.sql",
        artist_id,
        sc_user_id
    )
    .execute(&mut *connection)
    .await?;
    let primary_tracks = sqlx::query_file_scalar!(
        "queries/tracks/sc_account_detach_lock_primary.sql",
        artist_id,
        sc_user_id
    )
    .fetch_all(&mut *connection)
    .await?;
    sqlx::query_file!(
        "queries/tracks/sc_account_detach_clear_primary.sql",
        &primary_tracks
    )
    .execute(&mut *connection)
    .await?;
    Ok(json!({ "detached_tracks": detached }))
}
