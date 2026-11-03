use backend_contracts::admin_catalog::{ArtistCreate, ArtistUpdate};
use catalog_normalize::normalize_name;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::json;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::error::{Done, Refusal, encode};

#[derive(Serialize)]
struct ArtistRow {
    id: Uuid,
    name: String,
    normalized_name: String,
    country: Option<String>,
    avatar_url: Option<String>,
    bio: Option<String>,
    sc_user_id: Option<String>,
    source: String,
    confidence: f32,
    mb_artist_id: Option<String>,
    spotify_artist_id: Option<String>,
    genius_artist_id: Option<String>,
    merged_into: Option<Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

pub async fn create(connection: &mut PgConnection, body: &ArtistCreate) -> Done {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(Refusal::bad_request("name is required").into());
    }
    let normalized = normalize_name(name);
    if normalized.is_empty() {
        return Err(Refusal::bad_request("name normalizes to empty").into());
    }
    let exists = sqlx::query_file_scalar!("queries/artists/name_exists.sql", &normalized)
        .fetch_one(&mut *connection)
        .await?;
    if exists {
        return Err(Refusal::bad_request("artist with this name already exists").into());
    }
    let row = sqlx::query_file_as!(
        ArtistRow,
        "queries/artists/insert.sql",
        name,
        &normalized,
        body.country.as_deref(),
        body.bio.as_deref(),
        body.avatar_url.as_deref(),
        body.sc_user_id.as_deref()
    )
    .fetch_optional(&mut *connection)
    .await?
    .ok_or_else(|| Refusal::bad_request("artist with this name already exists"))?;
    encode(&row)
}

pub async fn update(connection: &mut PgConnection, artist_id: Uuid, body: &ArtistUpdate) -> Done {
    let name = body
        .name
        .as_ref()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());
    let normalized = name.as_deref().map(normalize_name);
    let row = sqlx::query_file_as!(
        ArtistRow,
        "queries/artists/update.sql",
        artist_id,
        name.as_deref(),
        normalized.as_deref(),
        body.country.as_deref(),
        body.bio.as_deref(),
        body.avatar_url.as_deref(),
        body.sc_user_id.as_deref(),
        body.confidence
    )
    .fetch_optional(&mut *connection)
    .await?
    .ok_or_else(|| Refusal::not_found("artist not found"))?;
    encode(&row)
}

pub async fn retry_crawl(connection: &mut PgConnection, artist_id: Uuid) -> Done {
    let reset = sqlx::query_file!("queries/artists/retry_crawl.sql", artist_id)
        .execute(&mut *connection)
        .await?;
    Ok(json!({ "reset": reset.rows_affected() }))
}

pub async fn merge(connection: &mut PgConnection, src: Uuid, dst: Uuid) -> Done {
    if src == dst {
        return Err(Refusal::bad_request("src and dst must differ").into());
    }
    sqlx::query_file!("queries/artists/merge_mark_artist.sql", dst, src)
        .execute(&mut *connection)
        .await?;

    sqlx::query_file!("queries/artists/merge_copy_track_artists.sql", dst, src)
        .execute(&mut *connection)
        .await?;
    let moved_track_artists =
        sqlx::query_file!("queries/artists/merge_delete_track_artists.sql", src)
            .execute(&mut *connection)
            .await?
            .rows_affected();

    sqlx::query_file!("queries/artists/merge_copy_album_artists.sql", dst, src)
        .execute(&mut *connection)
        .await?;
    let moved_album_artists =
        sqlx::query_file!("queries/artists/merge_delete_album_artists.sql", src)
            .execute(&mut *connection)
            .await?
            .rows_affected();

    sqlx::query_file!("queries/artists/merge_update_albums_primary.sql", dst, src)
        .execute(&mut *connection)
        .await?;
    sqlx::query_file!("queries/artists/merge_update_tracks_primary.sql", dst, src)
        .execute(&mut *connection)
        .await?;
    sqlx::query_file!("queries/artists/merge_update_wanted_primary.sql", dst, src)
        .execute(&mut *connection)
        .await?;

    sqlx::query_file!("queries/artists/merge_copy_sc_accounts.sql", dst, src)
        .execute(&mut *connection)
        .await?;
    let moved_sc_accounts = sqlx::query_file!("queries/artists/merge_delete_sc_accounts.sql", src)
        .execute(&mut *connection)
        .await?
        .rows_affected();

    sqlx::query_file!(
        "queries/artists/merge_copy_wanted_track_artists.sql",
        dst,
        src
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query_file!("queries/artists/merge_delete_wanted_track_artists.sql", src)
        .execute(&mut *connection)
        .await?;

    let coplay_dropped = sqlx::query_file!("queries/artists/merge_delete_coplay.sql", src)
        .execute(&mut *connection)
        .await?
        .rows_affected();

    Ok(json!({
        "moved_track_artists": moved_track_artists,
        "moved_album_artists": moved_album_artists,
        "moved_sc_accounts": moved_sc_accounts,
        "coplay_dropped": coplay_dropped,
    }))
}
