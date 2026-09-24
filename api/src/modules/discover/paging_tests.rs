use std::collections::HashSet;

use sqlx::PgPool;
use uuid::Uuid;

use super::{
    AlbumCursor, ArtistCursor, album_cursor_for_sort, artist_cursor_for_sort, cursor_row,
    fetch_albums, fetch_artists,
};

const ARTIST_SORTS: [&str; 6] = ["popular", "trending", "listeners", "tracks", "star", "az"];
const ALBUM_SORTS: [&str; 4] = ["popular", "recent", "tracks", "az"];
const TOTAL: i64 = 23;
const PAGE: i64 = 5;
const MAX_PAGES: usize = 40;

async fn seed_artists(pg: &PgPool) -> anyhow::Result<HashSet<Uuid>> {
    let mut ids = HashSet::new();
    for index in 0..TOTAL {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO artists (
                 id, name, normalized_name, source,
                 popularity_score, trending_score, monthly_listeners,
                 track_count_primary, is_star
             ) VALUES ($1, $2, lower($2), 'test', $3, $3, $4, $5, $6)",
        )
        .bind(id)
        .bind(format!("Artist {index:03}"))
        .bind(if index % 3 == 0 { 0.5_f32 } else { 0.1_f32 })
        .bind(if index % 4 == 0 { 100_i64 } else { 7_i64 })
        .bind(if index % 5 == 0 { 9_i32 } else { 2_i32 })
        .bind(index % 7 == 0)
        .execute(pg)
        .await?;
        ids.insert(id);
    }
    Ok(ids)
}

async fn seed_albums(pg: &PgPool) -> anyhow::Result<HashSet<Uuid>> {
    let artist = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO artists (id, name, normalized_name, source, track_count_primary)
         VALUES ($1, 'Album Maker', 'album maker', 'test', 9)",
    )
    .bind(artist)
    .execute(pg)
    .await?;
    let mut ids = HashSet::new();
    for index in 0..TOTAL {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO albums (
                 id, title, normalized_title, source, type, primary_artist_id,
                 track_count, popularity_score, release_year
             ) VALUES ($1, $2, lower($2), 'test', 'album', $6, $3, $4, $5)",
        )
        .bind(id)
        .bind(format!("Album {index:03}"))
        .bind(if index % 3 == 0 { 9_i32 } else { 2_i32 })
        .bind(if index % 4 == 0 { 0.5_f32 } else { 0.1_f32 })
        .bind(if index % 5 == 0 { 2020_i16 } else { 1999_i16 })
        .bind(artist)
        .execute(pg)
        .await?;
        ids.insert(id);
    }
    Ok(ids)
}

async fn walk_artists(pg: &PgPool, sort: &str) -> anyhow::Result<Vec<Uuid>> {
    let mut seen: Vec<Uuid> = Vec::new();
    let mut cursor: Option<ArtistCursor> = None;
    for _ in 0..MAX_PAGES {
        let rows = fetch_artists(pg, sort, None, None, cursor.as_ref(), PAGE + 1).await?;
        let next = cursor_row(rows.len(), PAGE)
            .and_then(|at| rows.get(at))
            .map(|row| artist_cursor_for_sort(sort, row));
        seen.extend(rows.into_iter().take(PAGE as usize).map(|row| row.id));
        match next {
            Some(next) => cursor = Some(next),
            None => return Ok(seen),
        }
    }
    anyhow::bail!("the walk over `{sort}` never reached the end")
}

async fn walk_albums(pg: &PgPool, sort: &str) -> anyhow::Result<Vec<Uuid>> {
    let mut seen: Vec<Uuid> = Vec::new();
    let mut cursor: Option<AlbumCursor> = None;
    for _ in 0..MAX_PAGES {
        let rows = fetch_albums(pg, sort, None, None, cursor.as_ref(), PAGE + 1).await?;
        let next = cursor_row(rows.len(), PAGE)
            .and_then(|at| rows.get(at))
            .map(|row| album_cursor_for_sort(sort, row));
        seen.extend(rows.into_iter().take(PAGE as usize).map(|row| row.id));
        match next {
            Some(next) => cursor = Some(next),
            None => return Ok(seen),
        }
    }
    anyhow::bail!("the walk over `{sort}` never reached the end")
}

fn assert_every_one_exactly_once(sort: &str, seen: &[Uuid], expected: &HashSet<Uuid>) {
    let unique: HashSet<Uuid> = seen.iter().copied().collect();
    assert_eq!(
        seen.len(),
        unique.len(),
        "sorted by `{sort}`, the walk handed the same entity out twice: {} pages worth of {} \
         rows hold only {} distinct ones",
        seen.len() / PAGE as usize,
        seen.len(),
        unique.len()
    );
    assert_eq!(
        &unique,
        expected,
        "sorted by `{sort}`, paging lost {} of {} entities on the way; a listener scrolling \
         this list never sees them at all",
        expected.difference(&unique).count(),
        expected.len()
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn every_artist_is_reached_exactly_once_whatever_the_sort(pg: PgPool) -> anyhow::Result<()> {
    let expected = seed_artists(&pg).await?;

    for sort in ARTIST_SORTS {
        let seen = walk_artists(&pg, sort).await?;
        assert_every_one_exactly_once(sort, &seen, &expected);
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn every_album_is_reached_exactly_once_whatever_the_sort(pg: PgPool) -> anyhow::Result<()> {
    let expected = seed_albums(&pg).await?;

    for sort in ALBUM_SORTS {
        let seen = walk_albums(&pg, sort).await?;
        assert_every_one_exactly_once(sort, &seen, &expected);
    }
    Ok(())
}

async fn pages_of_artists(pg: &PgPool) -> anyhow::Result<Vec<Vec<Uuid>>> {
    let mut pages: Vec<Vec<Uuid>> = Vec::new();
    let mut cursor: Option<ArtistCursor> = None;
    for _ in 0..MAX_PAGES {
        let rows = fetch_artists(pg, "popular", None, None, cursor.as_ref(), PAGE + 1).await?;
        let next = cursor_row(rows.len(), PAGE)
            .and_then(|at| rows.get(at))
            .map(|row| artist_cursor_for_sort("popular", row));
        pages.push(
            rows.into_iter()
                .take(PAGE as usize)
                .map(|row| row.id)
                .collect(),
        );
        match next {
            Some(next) => cursor = Some(next),
            None => return Ok(pages),
        }
    }
    anyhow::bail!("the walk never reached the end")
}

#[sqlx::test(migrations = "./migrations")]
async fn a_page_of_its_own_size_does_not_promise_another_one(pg: PgPool) -> anyhow::Result<()> {
    for index in 0..PAGE * 2 {
        sqlx::query(
            "INSERT INTO artists (
                 id, name, normalized_name, source, popularity_score, track_count_primary
             ) VALUES ($1, $2, lower($2), 'test', 0.5, 3)",
        )
        .bind(Uuid::now_v7())
        .bind(format!("Exactly {index:03}"))
        .execute(&pg)
        .await?;
    }

    let pages = pages_of_artists(&pg).await?;

    assert_eq!(
        pages.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![PAGE as usize, PAGE as usize],
        "ten artists at five a page is exactly two pages; a third one, or an empty tail, means \
         the pager promised a page it could not fill: {pages:?}"
    );
    Ok(())
}
