use chrono::{DateTime, NaiveDate, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppResult;
use crate::modules::playlists::{PlaylistRow, project_to_sc_shape as project_playlist};
use crate::modules::tracks::{TrackRow, project_to_sc_shape as project_track};
use crate::modules::users::{UserRow, project_to_sc_shape as project_user};

pub const STATEMENT_TIMEOUT_MS: i32 = 2500;

pub const TRIGRAM_MIN_LEN: usize = 3;

fn escape_like(q: &str) -> String {
    q.trim()
        .to_lowercase()
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub fn prefix_only(q: &str) -> bool {
    catalog_normalize::normalize_title(q).chars().count() < TRIGRAM_MIN_LEN
}

pub fn like_needle(q: &str) -> String {
    let lower = escape_like(q);
    if prefix_only(q) {
        format!("{lower}%")
    } else {
        format!("%{lower}%")
    }
}

pub fn like_needle_normalized(q: &str) -> String {
    let normalized = catalog_normalize::normalize_title(q);
    if prefix_only(q) {
        format!("{normalized}%")
    } else {
        format!("%{normalized}%")
    }
}

async fn set_statement_timeout(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> AppResult<()> {
    sqlx::query(&format!(
        "SET LOCAL statement_timeout = {STATEMENT_TIMEOUT_MS}"
    ))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn configure_catalog_search(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> AppResult<()> {
    sqlx::query_file!(
        "queries/search/repository/configure_catalog_search.sql",
        &STATEMENT_TIMEOUT_MS.to_string()
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(())
}

fn sole_term(terms: &[String]) -> Option<&str> {
    match terms {
        [only] => Some(only.as_str()),
        _ => None,
    }
}

pub struct TrackSearch<'a> {
    pub query: Option<&'a str>,
    pub owner: Option<&'a str>,
    pub ids: Option<&'a [String]>,
    pub genres: Option<&'a [String]>,
    pub tags: Option<&'a [String]>,
}

pub async fn search_tracks(
    pg: &PgPool,
    filters: &TrackSearch<'_>,
    page: i64,
    limit: i64,
) -> AppResult<(Vec<Value>, bool)> {
    let needle = filters.query.map(like_needle);
    let norm_needle = filters.query.map(like_needle_normalized);
    let prefix = filters.query.is_some_and(prefix_only);
    let offset = page * limit;

    let mut tx = pg.begin().await?;
    configure_catalog_search(&mut tx).await?;

    let fetch_limit = limit + 1;

    let sole_genre = filters.genres.and_then(sole_term);

    let rows: Vec<TrackRow> = match (filters.owner, filters.ids) {
        (Some(uid), Some(ids)) => {
            sqlx::query_file_as!(
                TrackRow,
                "queries/search/repository/search_tracks_by_uploader_ids.sql",
                uid,
                needle.as_deref(),
                fetch_limit,
                offset,
                norm_needle.as_deref(),
                ids,
                filters.genres,
                filters.tags,
                prefix
            )
            .fetch_all(&mut *tx)
            .await?
        }
        (Some(uid), None) => {
            sqlx::query_file_as!(
                TrackRow,
                "queries/search/repository/search_tracks_by_uploader.sql",
                uid,
                needle.as_deref(),
                fetch_limit,
                offset,
                norm_needle.as_deref(),
                filters.genres,
                sole_genre,
                filters.tags,
                prefix
            )
            .fetch_all(&mut *tx)
            .await?
        }
        (None, Some(ids)) => {
            sqlx::query_file_as!(
                TrackRow,
                "queries/search/repository/search_tracks_by_ids.sql",
                needle.as_deref(),
                fetch_limit,
                offset,
                norm_needle.as_deref(),
                ids,
                filters.genres,
                filters.tags,
                prefix
            )
            .fetch_all(&mut *tx)
            .await?
        }
        (None, None) => {
            sqlx::query_file_as!(
                TrackRow,
                "queries/search/repository/search_tracks_global.sql",
                needle.as_deref(),
                fetch_limit,
                offset,
                norm_needle.as_deref(),
                filters.genres,
                sole_genre,
                filters.tags,
                prefix
            )
            .fetch_all(&mut *tx)
            .await?
        }
    };

    tx.commit().await?;

    let has_more = rows.len() as i64 > limit;
    let rows: Vec<TrackRow> = rows.into_iter().take(limit as usize).collect();
    let projected = project_tracks_with_uploaders(pg, rows).await?;
    Ok((projected, has_more))
}

async fn project_tracks_with_uploaders(pg: &PgPool, rows: Vec<TrackRow>) -> AppResult<Vec<Value>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let uploader_ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r.uploader_sc_user_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let user_map: std::collections::HashMap<String, Value> = if uploader_ids.is_empty() {
        Default::default()
    } else {
        let users: Vec<UserRow> = sqlx::query_file_as!(
            UserRow,
            "queries/search/repository/users_by_sc_ids.sql",
            &uploader_ids
        )
        .fetch_all(pg)
        .await?;
        users
            .into_iter()
            .map(|u| (u.sc_user_id.clone(), project_user(&u)))
            .collect()
    };

    Ok(rows
        .into_iter()
        .map(|row| {
            let uploader = row
                .uploader_sc_user_id
                .as_deref()
                .and_then(|uid| user_map.get(uid));
            project_track(&row, uploader)
        })
        .collect())
}

pub async fn search_playlists(
    pg: &PgPool,
    q_lower: Option<&str>,
    user_sc_id_filter: Option<&str>,
    page: i64,
    limit: i64,
) -> AppResult<(Vec<Value>, bool)> {
    let needle = q_lower.map(like_needle);
    let norm_needle = q_lower.map(like_needle_normalized);
    let prefix = q_lower.is_some_and(prefix_only);
    let offset = page * limit;

    let mut tx = pg.begin().await?;
    configure_catalog_search(&mut tx).await?;

    let fetch_limit = limit + 1;

    let rows: Vec<PlaylistRow> = if let Some(uid) = user_sc_id_filter {
        sqlx::query_file_as!(
            PlaylistRow,
            "queries/search/repository/search_playlists_by_owner.sql",
            uid,
            needle.as_deref(),
            fetch_limit,
            offset,
            norm_needle.as_deref(),
            prefix
        )
        .fetch_all(&mut *tx)
        .await?
    } else {
        sqlx::query_file_as!(
            PlaylistRow,
            "queries/search/repository/search_playlists_global.sql",
            needle.as_deref(),
            fetch_limit,
            offset,
            norm_needle.as_deref(),
            prefix
        )
        .fetch_all(&mut *tx)
        .await?
    };

    tx.commit().await?;

    let has_more = rows.len() as i64 > limit;
    let rows: Vec<PlaylistRow> = rows.into_iter().take(limit as usize).collect();
    let projected = project_playlists_with_owners(pg, rows).await?;
    Ok((projected, has_more))
}

async fn project_playlists_with_owners(
    pg: &PgPool,
    rows: Vec<PlaylistRow>,
) -> AppResult<Vec<Value>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let owner_ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r.owner_sc_user_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let owner_map: std::collections::HashMap<String, Value> = if owner_ids.is_empty() {
        Default::default()
    } else {
        let users: Vec<UserRow> = sqlx::query_file_as!(
            UserRow,
            "queries/search/repository/users_by_sc_ids.sql",
            &owner_ids
        )
        .fetch_all(pg)
        .await?;
        users
            .into_iter()
            .map(|u| (u.sc_user_id.clone(), project_user(&u)))
            .collect()
    };

    Ok(rows
        .into_iter()
        .map(|row| {
            let owner = row
                .owner_sc_user_id
                .as_deref()
                .and_then(|uid| owner_map.get(uid));
            project_playlist(&row, owner)
        })
        .collect())
}

pub async fn search_users(
    pg: &PgPool,
    q_lower: Option<&str>,
    ids: Option<&[String]>,
    page: i64,
    limit: i64,
) -> AppResult<(Vec<Value>, bool)> {
    let needle = q_lower.map(like_needle);
    let norm_needle = q_lower.map(like_needle_normalized);
    let prefix = q_lower.is_some_and(prefix_only);
    let offset = page * limit;

    let mut tx = pg.begin().await?;
    configure_catalog_search(&mut tx).await?;

    let fetch_limit = limit + 1;

    let rows: Vec<UserRow> = sqlx::query_file_as!(
        UserRow,
        "queries/search/repository/search_users.sql",
        needle.as_deref(),
        fetch_limit,
        offset,
        ids,
        norm_needle.as_deref(),
        prefix
    )
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;

    let has_more = rows.len() as i64 > limit;
    let collection: Vec<Value> = rows
        .into_iter()
        .take(limit as usize)
        .map(|r| project_user(&r))
        .collect();
    Ok((collection, has_more))
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ArtistSearchRow {
    pub id: Uuid,
    pub name: String,
    pub country: Option<String>,
    pub avatar_url: Option<String>,
    pub confidence: f32,
    pub track_count_primary: i32,
    pub track_count_featured: i32,
    pub album_count_denorm: i32,
    pub monthly_listeners: i64,
    pub trending_score: f32,
    pub tags: Vec<String>,
    pub is_star: bool,
    pub star_aura_id: Option<String>,
    pub star_custom_hex: Option<String>,
}

pub async fn search_artists(
    pg: &PgPool,
    q_lower: &str,
    page: i64,
    limit: i64,
) -> AppResult<(Vec<ArtistSearchRow>, bool)> {
    let needle = like_needle(q_lower);
    let prefix = prefix_only(q_lower);
    let offset = page * limit;

    let mut tx = pg.begin().await?;
    set_statement_timeout(&mut tx).await?;

    let fetch_limit = limit + 1;

    let norm_needle = like_needle_normalized(q_lower);
    let rows: Vec<ArtistSearchRow> = sqlx::query_file_as!(
        ArtistSearchRow,
        "queries/search/repository/search_artists.sql",
        &needle,
        fetch_limit,
        offset,
        &norm_needle,
        prefix
    )
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;

    let has_more = rows.len() as i64 > limit;
    Ok((rows.into_iter().take(limit as usize).collect(), has_more))
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AlbumSearchRow {
    pub id: Uuid,
    pub title: String,
    pub kind: String,
    pub release_year: Option<i16>,
    pub release_date: Option<NaiveDate>,
    pub cover_url: Option<String>,
    pub confidence: f32,
    pub track_count: i32,
    pub total_duration_ms: i64,
    pub popularity_score: f32,
    pub is_star_artist: bool,
    pub primary_artist_id: Option<Uuid>,
    pub primary_artist_name: Option<String>,
    pub primary_artist_avatar: Option<String>,
}

pub async fn search_albums(
    pg: &PgPool,
    q_lower: &str,
    page: i64,
    limit: i64,
) -> AppResult<(Vec<AlbumSearchRow>, bool)> {
    let needle = like_needle(q_lower);
    let prefix = prefix_only(q_lower);
    let offset = page * limit;

    let mut tx = pg.begin().await?;
    set_statement_timeout(&mut tx).await?;

    let fetch_limit = limit + 1;

    let norm_needle = like_needle_normalized(q_lower);
    let rows: Vec<AlbumSearchRow> = sqlx::query_file_as!(
        AlbumSearchRow,
        "queries/search/repository/search_albums.sql",
        &needle,
        fetch_limit,
        offset,
        &norm_needle,
        prefix
    )
    .fetch_all(&mut *tx)
    .await?;

    tx.commit().await?;

    let has_more = rows.len() as i64 > limit;
    Ok((rows.into_iter().take(limit as usize).collect(), has_more))
}

#[allow(dead_code)]
pub async fn db_last_synced(pg: &PgPool) -> AppResult<Option<DateTime<Utc>>> {
    let row = sqlx::query_file_scalar!("queries/search/repository/db_last_synced.sql")
        .fetch_optional(pg)
        .await?;
    Ok(row.flatten())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seed(pool: &PgPool) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, genre, sharing,
                                 duration_ms, play_count_sc, uploader_sc_user_id)
             VALUES ('1', 'soundcloud:tracks:1', 'One',   'one',   'Drum & Bass', 'public', 1000, 10, '7'),
                    ('2', 'soundcloud:tracks:2', 'Two',   'two',   'DRUM & BASS', 'public', 1000, 30, '7'),
                    ('3', 'soundcloud:tracks:3', 'Three', 'three', 'Techno',      'public', 1000, 20, '7')",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    fn ids_of(rows: &[Value]) -> Vec<i64> {
        rows.iter()
            .filter_map(|row| row.get("id").and_then(|v| v.as_i64()))
            .collect()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn requested_ids_keep_their_request_order(pool: PgPool) -> anyhow::Result<()> {
        seed(&pool).await?;
        let ids = vec!["3".to_owned(), "1".to_owned(), "2".to_owned()];
        let filters = TrackSearch {
            query: None,
            owner: None,
            ids: Some(&ids),
            genres: None,
            tags: None,
        };
        let (rows, _) = search_tracks(&pool, &filters, 0, 10).await?;
        assert_eq!(ids_of(&rows), vec![3, 1, 2]);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_sole_genre_matches_exactly_what_the_multi_genre_path_matches(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        seed(&pool).await?;
        let sole = vec!["Drum & Bass".to_owned()];
        let pair = vec!["Drum & Bass".to_owned(), "drum & bass".to_owned()];

        let one = search_tracks(
            &pool,
            &TrackSearch {
                query: None,
                owner: None,
                ids: None,
                genres: Some(&sole),
                tags: None,
            },
            0,
            10,
        )
        .await?
        .0;
        let many = search_tracks(
            &pool,
            &TrackSearch {
                query: None,
                owner: None,
                ids: None,
                genres: Some(&pair),
                tags: None,
            },
            0,
            10,
        )
        .await?
        .0;

        assert_eq!(ids_of(&one), vec![2, 1]);
        assert_eq!(ids_of(&one), ids_of(&many));
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_sole_genre_stays_case_insensitive(pool: PgPool) -> anyhow::Result<()> {
        seed(&pool).await?;
        let shouted = vec!["DRUM & BASS".to_owned()];
        let whispered = vec!["drum & bass".to_owned()];
        let of = |genres: &Vec<String>| {
            let genres = genres.clone();
            let pool = pool.clone();
            async move {
                search_tracks(
                    &pool,
                    &TrackSearch {
                        query: None,
                        owner: None,
                        ids: None,
                        genres: Some(&genres),
                        tags: None,
                    },
                    0,
                    10,
                )
                .await
                .map(|(rows, _)| ids_of(&rows))
            }
        };
        assert_eq!(of(&shouted).await?, vec![2, 1]);
        assert_eq!(of(&whispered).await?, vec![2, 1]);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_sole_genre_page_is_served_in_index_order_without_a_sort(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, genre, sharing,
                                 duration_ms, play_count_sc)
             SELECT n::text,
                    'soundcloud:tracks:' || n,
                    'Track ' || n,
                    'track ' || n,
                    CASE
                        WHEN n % 10 = 0 THEN 'Drum & Bass'
                        ELSE (ARRAY['House', 'Techno', 'Hip-Hop', 'Trap'])[1 + n % 4]
                    END,
                    'public',
                    1000,
                    n
             FROM generate_series(1, 60000) AS n",
        )
        .execute(&pool)
        .await?;
        sqlx::query("ANALYZE tracks").execute(&pool).await?;

        let sql = include_str!("../../../queries/search/repository/search_tracks_global.sql");
        for offset in [0_i64, 600_i64] {
            let plan: Value = sqlx::query_scalar(&format!("EXPLAIN (FORMAT JSON) {sql}"))
                .bind(None::<String>)
                .bind(31_i64)
                .bind(offset)
                .bind(None::<String>)
                .bind(vec!["Drum & Bass".to_owned()])
                .bind("Drum & Bass")
                .bind(None::<Vec<String>>)
                .bind(false)
                .fetch_one(&pool)
                .await?;

            let plan = plan.to_string();
            assert!(
                plan.contains("tracks_public_genre_popular_idx"),
                "a sole-genre page at offset {offset} must be answered by the genre/popularity index: {plan}"
            );
            assert!(
                !plan.contains("\"Sort\""),
                "a sole-genre page at offset {offset} must not sort every matching track to return one page: {plan}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_short_query_searches_by_prefix_and_a_long_one_by_substring() {
        assert!(prefix_only("ne"));
        assert!(!prefix_only("nel"));
        assert_eq!(like_needle("ne"), "ne%");
        assert_eq!(like_needle("nel"), "%nel%");
        assert_eq!(like_needle_normalized("ne"), "ne%");
        assert_eq!(like_needle_normalized("nel"), "%nel%");
    }

    #[test]
    fn a_short_query_still_escapes_like_metacharacters() {
        assert_eq!(like_needle("_%"), "\\_\\%%");
        assert_eq!(like_needle("a_b"), "%a\\_b%");
    }

    #[test]
    fn the_normalized_needle_cannot_carry_a_wildcard_either() {
        for raw in ["100%", "a_b", "back\\slash", "%%%", "a%b_c\\d"] {
            let needle = like_needle_normalized(raw);
            let inside = needle.trim_start_matches('%').trim_end_matches('%');
            assert!(
                !inside.contains('%') && !inside.contains('_') && !inside.contains('\\'),
                "`{raw}` became `{needle}`; a metacharacter that survives normalization makes \
                 the query match the whole catalogue, and this path does not escape anything — \
                 it relies on normalization dropping them"
            );
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_short_query_never_seq_scans_the_track_table(pool: PgPool) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, sharing,
                                 duration_ms, play_count_sc, uploader_username)
             SELECT n::text,
                    'soundcloud:tracks:' || n,
                    CASE WHEN n % 1000 = 0 THEN 'Nelson ' || n ELSE 'Zulu ' || n END,
                    CASE WHEN n % 1000 = 0 THEN 'nelson ' || n ELSE 'zulu ' || n END,
                    'public', 1000, n, 'dj' || n
             FROM generate_series(1, 20000) AS n",
        )
        .execute(&pool)
        .await?;
        sqlx::query("ANALYZE tracks").execute(&pool).await?;

        let sql = include_str!("../../../queries/search/repository/search_tracks_global.sql");
        let plan: Value = sqlx::query_scalar(&format!("EXPLAIN (FORMAT JSON) {sql}"))
            .bind(like_needle("ne"))
            .bind(31_i64)
            .bind(0_i64)
            .bind(like_needle_normalized("ne"))
            .bind(None::<Vec<String>>)
            .bind(None::<String>)
            .bind(None::<Vec<String>>)
            .bind(prefix_only("ne"))
            .fetch_one(&pool)
            .await?;

        let plan = plan.to_string();
        assert!(
            !plan.contains("\"Seq Scan\""),
            "a two-character query must stay on the trigram index instead of reading every track: {plan}"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_uploader_page_keeps_the_requested_id_order(pool: PgPool) -> anyhow::Result<()> {
        seed(&pool).await?;
        let ids = vec!["2".to_owned(), "3".to_owned(), "1".to_owned()];
        let filters = TrackSearch {
            query: None,
            owner: Some("7"),
            ids: Some(&ids),
            genres: None,
            tags: None,
        };
        let (rows, _) = search_tracks(&pool, &filters, 0, 10).await?;
        assert_eq!(ids_of(&rows), vec![2, 3, 1]);
        Ok(())
    }
}

#[cfg(test)]
#[path = "popular_plan_tests.rs"]
mod popular_plan_tests;
