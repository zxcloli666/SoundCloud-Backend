use sqlx::{PgConnection, PgPool};
use uuid::Uuid;
use wt::{AlbumSlot, MatchAlbumTrack, MatchGeniusSong, MatchWantedLink};

pub async fn link_wanted_to_sc(
    pool: &PgPool,
    wanted_id: Uuid,
    sc_track_id: &str,
) -> Result<bool, sqlx::Error> {
    let link = MatchWantedLink {
        wanted_id,
        sc_track_id: sc_track_id.to_owned(),
        at: wt::now(),
    };
    wt::retry(|| async {
        let mut transaction = pool.begin().await?;
        let linked = link_in(&mut transaction, &link).await?;
        transaction.commit().await?;
        Ok(linked)
    })
    .await
}

async fn link_in(
    connection: &mut PgConnection,
    link: &MatchWantedLink,
) -> Result<bool, sqlx::Error> {
    let Some(track_id) = wt::call(&mut *connection, link).await? else {
        return Ok(false);
    };
    if let Some(genius_song_id) = crawled_genius_song_id(connection, link.wanted_id).await? {
        let attach = MatchGeniusSong {
            track_id,
            genius_song_id,
        };
        wt::call(&mut *connection, &attach).await?;
    }
    let albums: Vec<AlbumSlot> = sqlx::query_file!("queries/wanted_albums.sql", link.wanted_id)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|album| AlbumSlot {
            album_id: album.album_id,
            position: album.position,
        })
        .collect();
    if !albums.is_empty() {
        wt::call(connection, &MatchAlbumTrack { track_id, albums }).await?;
    }
    Ok(true)
}

async fn crawled_genius_song_id(
    connection: &mut PgConnection,
    wanted_id: Uuid,
) -> Result<Option<i64>, sqlx::Error> {
    let external_id = sqlx::query_file_scalar!("queries/wanted_genius_song_id.sql", wanted_id)
        .fetch_optional(connection)
        .await?;
    Ok(external_id.flatten().and_then(|id| id.parse().ok()))
}
