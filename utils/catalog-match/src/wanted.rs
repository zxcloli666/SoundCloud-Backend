use sqlx::PgPool;
use uuid::Uuid;

use crate::indexed::attach_genius_song;

pub async fn link_wanted_to_sc(
    pool: &PgPool,
    wanted_id: Uuid,
    sc_track_id: &str,
) -> Result<bool, sqlx::Error> {
    let linked = sqlx::query_file_scalar!("queries/link_to_indexed.sql", wanted_id, sc_track_id)
        .fetch_optional(pool)
        .await?;
    let Some(Some(track_id)) = linked else {
        return Ok(false);
    };

    if let Some(genius_song_id) = crawled_genius_song_id(pool, wanted_id).await? {
        attach_genius_song(pool, track_id, genius_song_id).await?;
    }
    inherit_albums(pool, wanted_id, track_id).await?;
    Ok(true)
}

async fn crawled_genius_song_id(
    pool: &PgPool,
    wanted_id: Uuid,
) -> Result<Option<i64>, sqlx::Error> {
    let external_id = sqlx::query_file_scalar!("queries/wanted_genius_song_id.sql", wanted_id)
        .fetch_optional(pool)
        .await?;
    Ok(external_id.flatten().and_then(|id| id.parse().ok()))
}

async fn inherit_albums(pool: &PgPool, wanted_id: Uuid, track_id: Uuid) -> Result<(), sqlx::Error> {
    let albums = sqlx::query_file!("queries/wanted_albums.sql", wanted_id)
        .fetch_all(pool)
        .await?;
    for album in albums {
        sqlx::query_file!(
            "queries/inherit_track_album.sql",
            track_id,
            album.album_id,
            album.position
        )
        .execute(pool)
        .await?;
        sqlx::query_file!(
            "queries/insert_album_track.sql",
            album.album_id,
            track_id,
            album.position
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}
