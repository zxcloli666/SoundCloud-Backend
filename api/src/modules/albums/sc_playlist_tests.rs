use sqlx::PgPool;
use uuid::Uuid;

const SC_PLAYLIST: &str = include_str!("../../../queries/albums/handlers/detail_sc_playlist.sql");

async fn album(pool: &PgPool, id: Uuid, kind: &str, sc_user_id: &str) -> anyhow::Result<()> {
    let artist = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source, sc_user_id)
         VALUES ($1, $2, $2, 'test', $2)",
    )
    .bind(artist)
    .bind(sc_user_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO albums (id, title, normalized_title, source, type, primary_artist_id)
         VALUES ($1, 'Night', 'night', 'test', $2, $3)",
    )
    .bind(id)
    .bind(kind)
    .bind(artist)
    .execute(pool)
    .await?;
    Ok(())
}

async fn playlist(
    pool: &PgPool,
    n: i32,
    owner: &str,
    kind: &str,
    track_count: i32,
) -> anyhow::Result<String> {
    let urn = format!("soundcloud:playlists:{n}");
    sqlx::query(
        "INSERT INTO playlists (urn, sc_playlist_id, title, title_normalized, owner_sc_user_id, playlist_type, track_count)
         VALUES ($1, $2, 'Night', 'night', $3, $4, $5)",
    )
    .bind(&urn)
    .bind(n.to_string())
    .bind(owner)
    .bind(kind)
    .bind(track_count)
    .execute(pool)
    .await?;
    Ok(urn)
}

async fn shared_track(
    pool: &PgPool,
    album: Uuid,
    playlist_urn: &str,
    n: i32,
) -> anyhow::Result<()> {
    let track = Uuid::new_v4();
    let sc_track_id = format!("t{n}-{playlist_urn}");
    sqlx::query(
        "INSERT INTO tracks (id, sc_track_id, urn, title, title_normalized, duration_ms)
         VALUES ($1, $2, $2, 't', 't', 1000)
         ON CONFLICT DO NOTHING",
    )
    .bind(track)
    .bind(&sc_track_id)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO album_tracks (album_id, track_id) VALUES ($1, $2)")
        .bind(album)
        .bind(track)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO playlist_track_projection (playlist_urn, sc_track_id, position) VALUES ($1, $2, 0)")
        .bind(playlist_urn)
        .bind(&sc_track_id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn found(pool: &PgPool, album: Uuid) -> anyhow::Result<Option<String>> {
    Ok(sqlx::query_scalar(SC_PLAYLIST)
        .bind(album)
        .fetch_optional(pool)
        .await?)
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_album_finds_its_own_playlist_by_owner_title_and_type(
    pool: PgPool,
) -> anyhow::Result<()> {
    let id = Uuid::from_u128(1);
    album(&pool, id, "album", "100").await?;
    playlist(&pool, 1, "100", "single", 1).await?;
    playlist(&pool, 2, "999", "album", 10).await?;
    let wanted = playlist(&pool, 3, "100", "album", 10).await?;

    assert_eq!(found(&pool, id).await?, Some(wanted));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_album_without_a_counterpart_has_no_playlist(pool: PgPool) -> anyhow::Result<()> {
    let id = Uuid::from_u128(1);
    album(&pool, id, "album", "100").await?;
    playlist(&pool, 1, "100", "single", 1).await?;
    playlist(&pool, 2, "999", "album", 10).await?;

    assert_eq!(found(&pool, id).await?, None);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_single_with_the_albums_title_is_not_the_album_and_overlap_picks_among_albums(
    pool: PgPool,
) -> anyhow::Result<()> {
    let single = Uuid::from_u128(1);
    album(&pool, single, "single", "100").await?;
    let single_playlist = playlist(&pool, 1, "100", "single", 1).await?;
    playlist(&pool, 2, "100", "album", 12).await?;
    assert_eq!(found(&pool, single).await?, Some(single_playlist));

    let full = Uuid::from_u128(2);
    album(&pool, full, "album", "200").await?;
    playlist(&pool, 3, "200", "album", 20).await?;
    let overlapping = playlist(&pool, 4, "200", "album", 5).await?;
    shared_track(&pool, full, &overlapping, 1).await?;
    assert_eq!(found(&pool, full).await?, Some(overlapping));
    Ok(())
}
