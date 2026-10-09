use std::sync::Arc;

use catalog_normalize::normalize_title;
use serde_json::Value;
use sqlx::PgPool;

use super::SearchService;
use crate::cache::CacheService;

struct Song {
    id: i64,
    title: String,
    artist: String,
    uploader: String,
    plays: i64,
    duration_ms: i32,
}

fn song(id: i64, title: &str, artist: &str, plays: i64) -> Song {
    Song {
        id,
        title: title.to_owned(),
        artist: artist.to_owned(),
        uploader: artist.to_owned(),
        plays,
        duration_ms: 200_000,
    }
}

fn service(pool: &PgPool) -> anyhow::Result<Arc<SearchService>> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(SearchService::new(pool.clone(), CacheService::new(redis)))
}

async fn insert(pool: &PgPool, song: &Song) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, metadata_artist,
                             uploader_username, uploader_sc_user_id, duration_ms, play_count_sc, sharing)
         VALUES ($1::bigint::text, 'soundcloud:tracks:' || $1, $2, $3, $4, $5,
                 (1000 + abs(hashtext($5)) % 100000)::text, $6, $7, 'public')",
    )
    .bind(song.id)
    .bind(&song.title)
    .bind(normalize_title(&song.title))
    .bind(&song.artist)
    .bind(&song.uploader)
    .bind(song.duration_ms)
    .bind(song.plays)
    .execute(pool)
    .await?;
    Ok(())
}

async fn seed(pool: &PgPool, songs: &[Song]) -> anyhow::Result<()> {
    for song in songs {
        insert(pool, song).await?;
    }
    let mut filler = 900_000;
    for song in songs {
        let words = format!(
            "{} {}",
            normalize_title(&song.title),
            song.artist.to_lowercase()
        );
        for word in words.split_whitespace() {
            filler += 1;
            insert(
                pool,
                &Song {
                    id: filler,
                    title: format!("{word} echo {filler}"),
                    artist: "filler".to_owned(),
                    uploader: "filler".to_owned(),
                    plays: 1,
                    duration_ms: 10_000 + filler as i32,
                },
            )
            .await?;
        }
    }
    refresh(pool).await
}

async fn refresh(pool: &PgPool) -> anyhow::Result<()> {
    super::lexicon_refresh::refresh(pool).await
}

fn ids(page: &[Value]) -> Vec<i64> {
    page.iter()
        .filter_map(|track| track["id"].as_i64())
        .collect()
}

async fn top(search: &SearchService, q: &str) -> anyhow::Result<Vec<i64>> {
    Ok(ids(&search.tracks(q, None, 0, 5).await?.collection))
}

fn catalog() -> Vec<Song> {
    vec![
        song(1, "Nothing Else Matters", "Metallica", 9_000_000),
        song(2, "Enter Sandman", "Metallica", 8_000_000),
        song(3, "Umbrella", "Rihanna", 7_000_000),
        song(4, "Umbrella (sped up)", "Rihanna", 9_500_000),
        song(5, "Кукла колдуна", "Король и Шут", 3_000_000),
        song(6, "Положение", "Скриптонит", 2_000_000),
        song(7, "Halo", "Beyoncé", 4_000_000),
        song(8, "Blinding Lights", "The Weeknd", 6_000_000),
        song(9, "Nothing Compares 2 U", "Sinead O'Connor", 1_000_000),
    ]
}

#[sqlx::test(migrations = "./migrations")]
async fn forgiving_queries_find_the_song(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool, &catalog()).await?;
    let search = service(&pool)?;
    for (q, expected) in [
        ("metalica nothing else maters", 1),
        ("rihanna umbrella official video", 3),
        ("nothing matters metallica", 1),
        ("matters else nothing", 1),
        ("kukla kolduna", 5),
        ("skriptonit", 6),
        ("скриптонит", 6),
        ("beyonce halo", 7),
        ("blinding lig", 8),
    ] {
        let found = top(&search, q).await?;
        assert_eq!(found.first(), Some(&expected), "{q} found {found:?}");
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_variant_ranks_below_the_original_unless_asked_for(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool, &catalog()).await?;
    let search = service(&pool)?;
    assert_eq!(top(&search, "umbrella").await?.first(), Some(&3));
    assert_eq!(top(&search, "umbrella sped up").await?.first(), Some(&4));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_fan_reupload_collapses_into_one_result(pool: PgPool) -> anyhow::Result<()> {
    let mut songs = catalog();
    songs.push(Song {
        uploader: "fanpage".to_owned(),
        plays: 10,
        duration_ms: 201_000,
        ..song(30, "Umbrella", "Rihanna", 0)
    });
    seed(&pool, &songs).await?;
    let found = top(&*service(&pool)?, "rihanna umbrella").await?;
    assert!(found.contains(&3), "{found:?}");
    assert!(!found.contains(&30), "{found:?}");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_exact_uploader_name_lists_their_uploads(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool, &catalog()).await?;
    sqlx::query(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized)
         SELECT uploader_sc_user_id, 'soundcloud:users:' || uploader_sc_user_id, 'Metallica', 'metallica'
         FROM tracks WHERE sc_track_id = '1'",
    )
    .execute(&pool)
    .await?;
    let found = top(&*service(&pool)?, "metallica").await?;
    let mut official = found[..2].to_vec();
    official.sort_unstable();
    assert_eq!(official, vec![1, 2], "{found:?}");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn hidden_tracks_never_surface(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool, &catalog()).await?;
    sqlx::query("UPDATE tracks SET sharing = 'private' WHERE sc_track_id = '1'")
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE tracks SET deleted_at = now() WHERE sc_track_id = '2'")
        .execute(&pool)
        .await?;
    sqlx::query(
        "UPDATE tracks SET superseded_by = (SELECT id FROM tracks WHERE sc_track_id = '4')
         WHERE sc_track_id = '3'",
    )
    .execute(&pool)
    .await?;
    let search = service(&pool)?;
    for q in [
        "nothing else matters",
        "enter sandman",
        "metallica",
        "umbrella",
    ] {
        let found = top(&search, q).await?;
        assert!(
            !found.iter().any(|id| [1, 2, 3].contains(id)),
            "{q} found {found:?}"
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn user_urn_scopes_tracks_and_playlists(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool, &catalog()).await?;
    let owner: String =
        sqlx::query_scalar("SELECT uploader_sc_user_id FROM tracks WHERE sc_track_id = '2'")
            .fetch_one(&pool)
            .await?;
    sqlx::query(
        "INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id, owner_username)
         VALUES ('71', 'soundcloud:playlists:71', 'Sandman Sessions', 'sandman sessions', $1, 'Metallica'),
                ('72', 'soundcloud:playlists:72', 'Sandman Covers', 'sandman covers', '5', 'someone')",
    )
    .bind(&owner)
    .execute(&pool)
    .await?;
    refresh(&pool).await?;
    let search = service(&pool)?;
    let urn = format!("soundcloud:users:{owner}");
    let scoped = search.tracks("sandman", Some(&urn), 0, 10).await?;
    assert_eq!(ids(&scoped.collection), vec![2]);
    assert!(
        search
            .tracks("umbrella", Some(&owner), 0, 10)
            .await?
            .collection
            .is_empty()
    );
    let playlists = search.playlists("sandman", Some(&owner), 0, 10).await?;
    assert_eq!(playlists.collection.len(), 1);
    assert_eq!(playlists.collection[0]["urn"], "soundcloud:playlists:71");
    assert!(
        search
            .tracks("", Some(&owner), 0, 10)
            .await?
            .collection
            .is_empty()
    );
    assert!(
        search
            .tracks("sandman", Some("soundcloud:tracks:1"), 0, 10)
            .await?
            .collection
            .is_empty()
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn users_match_full_names_and_playlists_match_owners(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized, full_name, followers_count)
         VALUES ('17', 'soundcloud:users:17', 'djshadow', 'djshadow', 'Joshua Davis', 10),
                ('18', 'soundcloud:users:18', 'joshua', 'joshua', 'Someone Else', 5)",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id, owner_username)
         VALUES ('42', 'soundcloud:playlists:42', 'Late Night', 'late night', '17', 'djshadow'),
                ('43', 'soundcloud:playlists:43', 'Morning', 'morning', '18', 'joshua')",
    )
    .execute(&pool)
    .await?;
    let search = service(&pool)?;
    let users = search.users("davis", 0, 10).await?;
    assert_eq!(users.collection[0]["username"], "djshadow");
    let playlists = search.playlists("djshadow", None, 0, 10).await?;
    assert_eq!(playlists.collection.len(), 1);
    assert_eq!(playlists.collection[0]["user"]["username"], "djshadow");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn artists_without_tracks_and_empty_albums_stay_hidden(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source, track_count_primary, monthly_listeners)
         VALUES ('00000000-0000-0000-0000-000000000001', 'Massive Attack', 'massive attack', 'test', 12, 100),
                ('00000000-0000-0000-0000-000000000002', 'Massive Ghost', 'massive ghost', 'test', 0, 900)",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO albums (id, title, normalized_title, source, type, primary_artist_id, track_count)
         VALUES ('00000000-0000-0000-0000-00000000000a', 'Mezzanine', 'mezzanine', 'test', 'album',
                 '00000000-0000-0000-0000-000000000001', 11),
                ('00000000-0000-0000-0000-00000000000b', 'Mezzanine Demos', 'mezzanine demos', 'test', 'album',
                 '00000000-0000-0000-0000-000000000001', 0)",
    )
    .execute(&pool)
    .await?;
    let search = service(&pool)?;
    let artists = search.artists("massive", 0, 10).await?;
    assert_eq!(artists.collection.len(), 1);
    assert_eq!(artists.collection[0]["name"], "Massive Attack");
    let albums = search.albums("mezzanine", 0, 10).await?;
    assert_eq!(albums.collection.len(), 1);
    assert_eq!(
        albums.collection[0]["primary_artist"]["name"],
        "Massive Attack"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_last_page_never_promises_another(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing)
         SELECT id::text, 'soundcloud:tracks:' || id, 'Boundary ' || id, 'boundary ' || id,
                id * 10000, 'public'
         FROM generate_series(1, 30) id",
    )
    .execute(&pool)
    .await?;
    refresh(&pool).await?;
    let search = service(&pool)?;
    for (requested, served, more) in [
        (0, 0, true),
        (23, 23, true),
        (24, 24, false),
        (99, 24, false),
    ] {
        let page = search.tracks("boundary", None, requested, 1).await?;
        assert_eq!(
            (page.page, page.has_more),
            (served, more),
            "page {requested}"
        );
        assert_eq!(page.collection.len(), 1);
    }
    let short = search.tracks("a", None, -3, 999).await?;
    assert_eq!(
        (short.page, short.page_size, short.has_more),
        (0, 50, false)
    );
    Ok(())
}
