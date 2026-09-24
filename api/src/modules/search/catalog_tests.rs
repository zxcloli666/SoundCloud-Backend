use super::SearchService;
use super::query::{PlaylistSearchQuery, TrackSearchQuery};
use crate::cache::CacheService;
use serde_json::json;
use sqlx::PgPool;

fn service(pool: &PgPool) -> anyhow::Result<std::sync::Arc<SearchService>> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(SearchService::new(pool.clone(), CacheService::new(redis)))
}

#[sqlx::test(migrations = "./migrations")]
async fn raw_handles_and_normalized_titles_remain_searchable(pool: PgPool) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, uploader_username, genre)
        VALUES ('42', 'soundcloud:tracks:42', 'Ёлки', 'елки', 120000, 'dj_shadow', 'ЭЛЕКТРОНИКА')")
        .execute(&pool).await?;
    sqlx::query(
        "INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, owner_username)
        VALUES ('42', 'soundcloud:playlists:42', 'Ёлки', 'елки', 'dj_shadow')",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized)
        VALUES ('17', 'soundcloud:users:17', 'dj_shadow', 'dj shadow')",
    )
    .execute(&pool)
    .await?;
    let search = service(&pool)?;
    for q in ["dj_shadow", "ёлки", "елки"] {
        assert_eq!(
            search
                .tracks(
                    &TrackSearchQuery {
                        q: Some(q.into()),
                        ..Default::default()
                    },
                    0,
                    30
                )
                .await?
                .collection
                .len(),
            1
        );
        assert_eq!(
            search
                .playlists(
                    &PlaylistSearchQuery {
                        q: Some(q.into()),
                        ..Default::default()
                    },
                    0,
                    30
                )
                .await?
                .collection
                .len(),
            1
        );
    }
    for q in ["dj_shadow", "dj shadow"] {
        assert_eq!(search.users(q, None, 0, 30).await?.collection.len(), 1);
    }
    assert!(
        search
            .tracks(
                &TrackSearchQuery {
                    q: Some("dj%shadow".into()),
                    ..Default::default()
                },
                0,
                30
            )
            .await?
            .collection
            .is_empty()
    );
    assert_eq!(
        search
            .tracks(
                &TrackSearchQuery {
                    genres: Some("электроника".into()),
                    ..Default::default()
                },
                0,
                30
            )
            .await?
            .collection
            .len(),
        1
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn last_allowed_page_never_advertises_an_unreachable_next_page(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
        SELECT id::text, 'soundcloud:tracks:' || id, 'Boundary', 'boundary', 120000 FROM generate_series(1, 26) id")
        .execute(&pool).await?;
    sqlx::query("INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized)
        SELECT id::text, 'soundcloud:playlists:' || id, 'Boundary', 'boundary' FROM generate_series(1, 26) id")
        .execute(&pool).await?;
    sqlx::query("INSERT INTO users (sc_user_id, urn, username, username_normalized)
        SELECT id::text, 'soundcloud:users:' || id, 'Boundary', 'boundary' FROM generate_series(1, 26) id")
        .execute(&pool).await?;
    let search = service(&pool)?;
    let tracks = TrackSearchQuery {
        q: Some("boundary".into()),
        ..Default::default()
    };
    let playlists = PlaylistSearchQuery {
        q: Some("boundary".into()),
        ..Default::default()
    };
    for requested_page in [23, 24, 25] {
        for result in [
            search.tracks(&tracks, requested_page, 1).await?,
            search.playlists(&playlists, requested_page, 1).await?,
            search.users("boundary", None, requested_page, 1).await?,
        ] {
            assert_eq!(result.page, requested_page.min(24));
            assert_eq!(result.collection.len(), 1);
            assert_eq!(result.has_more, requested_page < 24);
        }
    }
    for result in [
        search.artists("", -1, 999).await?,
        search.albums("", -1, 999).await?,
    ] {
        assert_eq!(
            (result.page, result.page_size, result.has_more),
            (0, 50, false)
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn track_filters_intersect_and_visibility_is_applied_before_pagination(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, uploader_sc_user_id, genre, tags, play_count_sc)
        SELECT id::text, 'soundcloud:tracks:' || id, 'Song ' || id, 'song ' || id,
               120000, '17', 'Rock', ARRAY['dub','electro'], id
        FROM generate_series(42, 46) id")
        .execute(&pool).await?;
    sqlx::query("UPDATE tracks SET sharing = 'private' WHERE sc_track_id = '44'")
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE tracks SET deleted_at = now() WHERE sc_track_id = '45'")
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE tracks SET superseded_by = (SELECT id FROM tracks WHERE sc_track_id = '42') WHERE sc_track_id = '46'").execute(&pool).await?;
    let search = service(&pool)?;
    let mut query = TrackSearchQuery {
        q: Some("song".into()),
        ids: Some("43,42,44,45,46".into()),
        genres: Some("ROCK".into()),
        tags: Some("dub,missing".into()),
        user_urn: Some("soundcloud:users:17".into()),
        ..Default::default()
    };
    let first = search.tracks(&query, 0, 1).await?;
    assert_eq!(first.collection[0]["id"], 43);
    assert!(first.has_more);
    let second = search.tracks(&query, 1, 1).await?;
    assert_eq!(second.collection[0]["id"], 42);
    assert!(!second.has_more);
    query.genres = Some("Jazz".into());
    assert!(search.tracks(&query, 0, 50).await?.collection.is_empty());
    query.genres = None;
    query.tags = Some("unknown".into());
    assert!(search.tracks(&query, 0, 50).await?.collection.is_empty());
    query.tags = None;
    query.user_urn = Some("18".into());
    assert!(search.tracks(&query, 0, 50).await?.collection.is_empty());
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn global_track_search_reads_changes_and_accepts_filters_without_a_name(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, genre, tags)
        VALUES ('42', 'soundcloud:tracks:42', 'Original song', 'original song', 120000, 'Rock', ARRAY['dub'])")
        .execute(&pool).await?;
    let search = service(&pool)?;
    for query in [
        TrackSearchQuery {
            q: Some("original".into()),
            ..Default::default()
        },
        TrackSearchQuery {
            ids: Some("soundcloud:tracks:42,42,999".into()),
            ..Default::default()
        },
        TrackSearchQuery {
            genres: Some("rock".into()),
            ..Default::default()
        },
        TrackSearchQuery {
            tags: Some("dub".into()),
            ..Default::default()
        },
    ] {
        assert_eq!(search.tracks(&query, 0, 30).await?.collection.len(), 1);
    }
    let query = TrackSearchQuery {
        q: Some("original".into()),
        ..Default::default()
    };
    sqlx::query("UPDATE tracks SET sharing = 'private' WHERE sc_track_id = '42'")
        .execute(&pool)
        .await?;
    assert!(search.tracks(&query, 0, 30).await?.collection.is_empty());
    sqlx::query(
        "UPDATE tracks SET sharing = 'public', deleted_at = now() WHERE sc_track_id = '42'",
    )
    .execute(&pool)
    .await?;
    assert!(search.tracks(&query, 0, 30).await?.collection.is_empty());
    let empty = search.tracks(&TrackSearchQuery::default(), -1, 999).await?;
    assert!(empty.collection.is_empty());
    assert_eq!((empty.page, empty.page_size), (0, 50));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn playlist_search_stays_local_and_never_returns_private_or_deleted_items(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query("INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, owner_sc_user_id, likes_count_sc)
        SELECT id::text, 'soundcloud:playlists:' || id, 'Mix ' || id, 'mix ' || id, '17', id
        FROM generate_series(42, 45) id")
        .execute(&pool).await?;
    sqlx::query("UPDATE playlists SET sharing = 'private' WHERE sc_playlist_id = '45'")
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE playlists SET deleted_at = now() WHERE sc_playlist_id = '44'")
        .execute(&pool)
        .await?;
    let search = service(&pool)?;
    let mut query = PlaylistSearchQuery {
        q: Some("mix".into()),
        ..Default::default()
    };
    let page = search.playlists(&query, 0, 1).await?;
    assert_eq!(page.collection[0]["id"], 43);
    assert!(page.has_more);
    query.user_urn = Some("17".into());
    let page = search.playlists(&query, 1, 1).await?;
    assert_eq!(page.collection[0]["id"], 42);
    assert!(!page.has_more);
    query.q = None;
    assert_eq!(search.playlists(&query, 0, 30).await?.collection.len(), 2);
    sqlx::query("UPDATE playlists SET sharing = 'private' WHERE sc_playlist_id = '42'")
        .execute(&pool)
        .await?;
    let page = search.playlists(&query, 0, 30).await?;
    assert_eq!(page.collection.len(), 1);
    assert_eq!(page.collection[0]["id"], json!(43));
    assert!(page.collection[0].get("tracks").is_none());
    query.show_tracks = Some("true".into());
    assert!(search.playlists(&query, 0, 30).await.is_err());
    query.show_tracks = Some("false".into());
    query.access = Some("playable,preview,blocked".into());
    assert_eq!(search.playlists(&query, 0, 30).await?.collection.len(), 1);
    query.access = Some("playable".into());
    assert!(search.playlists(&query, 0, 30).await.is_err());
    Ok(())
}

async fn seed_catalog_entities(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO artists (
             id, name, normalized_name, source, track_count_primary, monthly_listeners
         ) VALUES
             ('00000000-0000-4000-9000-000000000001', 'Daft Punk', 'daft punk', 'test', 4, 900),
             ('00000000-0000-4000-9000-000000000002', 'Daft Sailor', 'daft sailor', 'test', 2, 100),
             ('00000000-0000-4000-9000-000000000003', 'Unrelated', 'unrelated', 'test', 9, 5000),
             ('00000000-0000-4000-9000-000000000004', 'Silent Daft', 'silent daft', 'test', 0, 7000)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO albums (
             id, title, normalized_title, source, track_count, popularity_score, primary_artist_id
         ) VALUES
             ('00000000-0000-4000-9001-000000000001', 'Discovery', 'discovery', 'test', 14, 0.9,
              '00000000-0000-4000-9000-000000000001'),
             ('00000000-0000-4000-9001-000000000002', 'Discovery Live', 'discovery live', 'test', 3, 0.2,
              '00000000-0000-4000-9000-000000000002'),
             ('00000000-0000-4000-9001-000000000003', 'Nothing', 'nothing', 'test', 5, 0.5, NULL),
             ('00000000-0000-4000-9001-000000000004', 'Discovery Empty', 'discovery empty', 'test', 0, 1.0, NULL)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn artist_search_reads_postgres_and_hides_artists_without_tracks(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_catalog_entities(&pool).await?;
    let search = service(&pool)?;

    let page = search.artists("daft", 0, 30).await?;

    let names: Vec<String> = page
        .collection
        .iter()
        .map(|artist| artist["name"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(names, vec!["Daft Punk", "Daft Sailor"]);
    assert!(!page.has_more);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn album_search_carries_its_artist_and_skips_empty_releases(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_catalog_entities(&pool).await?;
    let search = service(&pool)?;

    let page = search.albums("discovery", 0, 30).await?;

    let titles: Vec<String> = page
        .collection
        .iter()
        .map(|album| album["title"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(titles, vec!["Discovery", "Discovery Live"]);
    assert_eq!(page.collection[0]["primary_artist"]["name"], "Daft Punk");
    assert_eq!(page.collection[1]["primary_artist"]["name"], "Daft Sailor");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_dead_cache_never_hides_catalog_entities(pool: PgPool) -> anyhow::Result<()> {
    seed_catalog_entities(&pool).await?;
    let search = service(&pool)?;

    let artists = search.artists("daft", 0, 30).await?;
    let albums = search.albums("discovery", 0, 30).await?;

    assert_eq!(artists.collection.len(), 2);
    assert_eq!(albums.collection.len(), 2);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn catalog_search_pages_are_bounded_and_end_cleanly(pool: PgPool) -> anyhow::Result<()> {
    seed_catalog_entities(&pool).await?;
    let search = service(&pool)?;

    let first = search.artists("daft", 0, 1).await?;
    let second = search.artists("daft", 1, 1).await?;
    let past_end = search.artists("daft", 2, 1).await?;

    assert!(first.has_more);
    assert!(!second.has_more);
    assert!(past_end.collection.is_empty());
    assert_eq!(first.collection[0]["name"], "Daft Punk");
    assert_eq!(second.collection[0]["name"], "Daft Sailor");
    Ok(())
}

fn cached_service(pool: &PgPool) -> anyhow::Result<std::sync::Arc<SearchService>> {
    let redis = deadpool_redis::Config::from_url(
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned()),
    )
    .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(SearchService::new(pool.clone(), CacheService::new(redis)))
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_catalog_page_served_from_the_cache_is_the_page_that_was_stored(
    pool: PgPool,
) -> anyhow::Result<()> {
    let name = format!("Cachable {}", std::process::id());
    let normalized = name.to_lowercase();
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source, track_count_primary)
         VALUES (gen_random_uuid(), $1, $2, 'test', 3)",
    )
    .bind(&name)
    .bind(&normalized)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO albums (id, title, normalized_title, source, track_count)
         VALUES (gen_random_uuid(), $1, $2, 'test', 4)",
    )
    .bind(&name)
    .bind(&normalized)
    .execute(&pool)
    .await?;

    let search = cached_service(&pool)?;
    let cold_artists = search.artists(&name, 0, 10).await?;
    let warm_artists = search.artists(&name, 0, 10).await?;
    let cold_albums = search.albums(&name, 0, 10).await?;
    let warm_albums = search.albums(&name, 0, 10).await?;

    assert_eq!(
        cold_artists.collection.len(),
        1,
        "the artist was seeded for this run alone and must be found: {cold_artists:?}"
    );
    assert_eq!(
        cold_albums.collection.len(),
        1,
        "the album was seeded for this run alone and must be found: {cold_albums:?}"
    );
    assert_eq!(
        serde_json::to_value(&warm_artists)?,
        serde_json::to_value(&cold_artists)?,
        "the second ask is served from Redis, and it came back different from what was stored"
    );
    assert_eq!(
        serde_json::to_value(&warm_albums)?,
        serde_json::to_value(&cold_albums)?,
        "the second ask is served from Redis, and it came back different from what was stored"
    );
    Ok(())
}
