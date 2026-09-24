use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::common::session::SessionCtx;
use crate::error::{AppError, AppResult};
use crate::modules::discover::cursor::{self, AlbumCursor, ArtistCursor};
use crate::modules::discover::service::CachedTagList;
use crate::modules::discover::tags::{canonicalize_tag, canonicalize_tags};
use crate::state::AppState;

#[cfg(test)]
#[path = "paging_tests.rs"]
mod paging_tests;

const DEFAULT_LIMIT: i64 = 80;
const MAX_LIMIT: i64 = 200;
const MIN_SEARCH_LEN: usize = 2;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/discover/artists", get(artists))
        .route("/discover/albums", get(albums))
        .route("/discover/albums/by-year", get(albums_by_year))
        .route("/discover/spotlight", get(spotlight))
        .route("/discover/summary", get(summary))
        .route("/discover/random", get(random))
        .route("/discover/tags", get(tags))
}

#[derive(Debug, Deserialize)]
struct ArtistsQuery {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    sort: Option<String>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    q: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AlbumsQuery {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    sort: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    q: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SpotlightQuery {
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct AlbumsByYearQuery {
    #[serde(default)]
    years: Option<i64>,
    #[serde(default)]
    per_year: Option<i64>,
    #[serde(default)]
    kind: Option<String>,
}

#[derive(Debug, Serialize)]
struct YearBucket {
    year: i32,
    items: Vec<CatalogAlbum>,
}

#[derive(Debug, Serialize)]
struct YearBucketsResponse {
    buckets: Vec<YearBucket>,
}

#[derive(Debug, Deserialize)]
struct RandomQuery {
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

#[derive(Debug, Serialize)]
struct CatalogArtist {
    id: Uuid,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    avatar_url: Option<String>,
    confidence: f32,
    track_count_primary: i32,
    track_count_featured: i32,
    album_count: i32,
    monthly_listeners: i64,
    trending: f32,
    popularity: f32,
    tags: Vec<String>,
    star: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    aura_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    custom_hex: Option<String>,
}

#[derive(Debug, Serialize)]
struct CatalogAlbumArtist {
    id: Uuid,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    avatar_url: Option<String>,
}

#[derive(Debug, Serialize)]
struct CatalogAlbum {
    id: Uuid,
    title: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    release_year: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    release_month: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cover_url: Option<String>,
    confidence: f32,
    primary_artist: CatalogAlbumArtist,
    track_count: i32,
    total_duration_ms: i64,
    popularity: f32,
    star: bool,
}

#[derive(Debug, Serialize)]
struct ListResponse<T> {
    items: Vec<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
struct DiscoverSummary {
    artists_count: i64,
    albums_count: i64,
    fresh_count: i64,
    fresh_window_days: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ArtistRow {
    id: Uuid,
    name: String,
    normalized_name: String,
    country: Option<String>,
    avatar_url: Option<String>,
    confidence: f32,
    track_count_primary: i32,
    track_count_featured: i32,
    album_count_denorm: i32,
    monthly_listeners: i64,
    trending_score: f32,
    popularity_score: f32,
    tags: Vec<String>,
    is_star: bool,
    star_aura_id: Option<String>,
    star_custom_hex: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct AlbumRow {
    id: Uuid,
    title: String,
    normalized_title: String,
    kind: String,
    release_year: Option<i16>,
    release_date: Option<NaiveDate>,
    cover_url: Option<String>,
    confidence: f32,
    track_count: i32,
    total_duration_ms: i64,
    popularity_score: f32,
    is_star_artist: bool,
    primary_artist_id: Option<Uuid>,
    primary_artist_name: Option<String>,
    primary_artist_avatar: Option<String>,
}

fn resolved_limit(req: Option<i64>) -> i64 {
    req.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

fn usable_search(s: &str) -> Option<&str> {
    let trimmed = s.trim();
    if trimmed.chars().count() < MIN_SEARCH_LEN {
        return None;
    }
    Some(trimmed)
}

fn artist_sort_kind(s: Option<&str>) -> &'static str {
    match s.unwrap_or("popular") {
        "trending" => "trending",
        "listeners" => "listeners",
        "tracks" => "tracks",
        "star" => "star",
        "az" => "az",
        _ => "popular",
    }
}

fn album_sort_kind(s: Option<&str>) -> &'static str {
    match s.unwrap_or("popular") {
        "recent" => "recent",
        "tracks" => "tracks",
        "az" => "az",
        _ => "popular",
    }
}

fn album_kind_filter(s: Option<&str>) -> Option<&'static str> {
    match s.unwrap_or("all") {
        "album" => Some("album"),
        "ep" => Some("ep"),
        "single" => Some("single"),
        "compilation" => Some("compilation"),
        _ => None,
    }
}

fn artist_cursor_for_sort(sort: &str, row: &ArtistRow) -> ArtistCursor {
    let (p, p2) = match sort {
        "trending" => (row.trending_score as f64, 0.0),
        "listeners" => (row.monthly_listeners as f64, 0.0),
        "tracks" => (row.track_count_primary as f64, 0.0),
        "star" => (
            if row.is_star { 1.0 } else { 0.0 },
            row.trending_score as f64,
        ),
        "az" => (0.0, 0.0),
        _ => (row.popularity_score as f64, 0.0),
    };
    ArtistCursor {
        p,
        p2,
        n: row.normalized_name.clone(),
        id: row.id,
    }
}

fn cursor_row(fetched: usize, limit: i64) -> Option<usize> {
    let limit = usize::try_from(limit).ok()?;
    (fetched > limit && limit > 0).then_some(limit - 1)
}

fn epoch() -> NaiveDate {
    NaiveDate::from_ymd_opt(1970, 1, 1).expect("static date 1970-01-01")
}

fn release_ordering_days(row: &AlbumRow) -> f64 {
    let ordered_by = row.release_date.unwrap_or_else(|| {
        NaiveDate::from_ymd_opt(row.release_year.map(i32::from).unwrap_or(1970), 1, 1)
            .unwrap_or_else(epoch)
    });
    ordered_by.signed_duration_since(epoch()).num_days() as f64
}

fn album_cursor_for_sort(sort: &str, row: &AlbumRow) -> AlbumCursor {
    let (p, p2) = match sort {
        "popular" => (row.popularity_score as f64, 0.0),
        "tracks" => (row.track_count as f64, 0.0),
        "az" => (0.0, 0.0),
        _ => (
            row.release_year.unwrap_or(0) as f64,
            release_ordering_days(row),
        ),
    };
    AlbumCursor {
        p,
        p2,
        n: row.normalized_title.clone(),
        id: row.id,
    }
}

async fn fetch_artists(
    pg: &PgPool,
    sort: &str,
    tag: Option<&str>,
    search: Option<&str>,
    cursor: Option<&ArtistCursor>,
    limit: i64,
) -> AppResult<Vec<ArtistRow>> {
    let order_clause = match sort {
        "trending" => "trending_score DESC, normalized_name ASC, id ASC",
        "listeners" => "monthly_listeners DESC, normalized_name ASC, id ASC",
        "tracks" => "track_count_primary DESC, normalized_name ASC, id ASC",
        "star" => "is_star DESC, trending_score DESC, normalized_name ASC, id ASC",
        "az" => "normalized_name ASC, id ASC",
        _ => "popularity_score DESC, normalized_name ASC, id ASC",
    };

    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT id, name, normalized_name, country, avatar_url, confidence, \
                track_count_primary, track_count_featured, album_count_denorm, \
                monthly_listeners, trending_score, popularity_score, tags, \
                is_star, star_aura_id, star_custom_hex \
         FROM artists \
         WHERE merged_into IS NULL \
           AND (track_count_primary > 0 OR track_count_featured > 0)",
    );

    if let Some(t) = tag {
        qb.push(" AND tags @> ARRAY[")
            .push_bind(t.to_string())
            .push("]::text[]");
    }

    if let Some(q) = search {
        let needle = format!("%{}%", q.trim().to_lowercase());
        qb.push(" AND (normalized_name LIKE ")
            .push_bind(needle.clone())
            .push(" OR LOWER(name) LIKE ")
            .push_bind(needle.clone())
            .push(" OR EXISTS(SELECT 1 FROM unnest(tags) tg WHERE LOWER(tg) LIKE ")
            .push_bind(needle)
            .push("))");
    }

    if let Some(c) = cursor {
        match sort {
            "listeners" => {
                let p = c.p as i64;
                qb.push(" AND (monthly_listeners < ")
                    .push_bind(p)
                    .push(" OR (monthly_listeners = ")
                    .push_bind(p)
                    .push(" AND normalized_name > ")
                    .push_bind(c.n.clone())
                    .push(") OR (monthly_listeners = ")
                    .push_bind(p)
                    .push(" AND normalized_name = ")
                    .push_bind(c.n.clone())
                    .push(" AND id > ")
                    .push_bind(c.id)
                    .push("))");
            }
            "tracks" => {
                let p = c.p as i32;
                qb.push(" AND (track_count_primary < ")
                    .push_bind(p)
                    .push(" OR (track_count_primary = ")
                    .push_bind(p)
                    .push(" AND normalized_name > ")
                    .push_bind(c.n.clone())
                    .push(") OR (track_count_primary = ")
                    .push_bind(p)
                    .push(" AND normalized_name = ")
                    .push_bind(c.n.clone())
                    .push(" AND id > ")
                    .push_bind(c.id)
                    .push("))");
            }
            "star" => {
                let p_star = c.p as i32;
                let p_trend = c.p2 as f32;
                qb.push(" AND (is_star::int < ")
                    .push_bind(p_star)
                    .push(" OR (is_star::int = ")
                    .push_bind(p_star)
                    .push(" AND trending_score < ")
                    .push_bind(p_trend)
                    .push(") OR (is_star::int = ")
                    .push_bind(p_star)
                    .push(" AND trending_score = ")
                    .push_bind(p_trend)
                    .push(" AND normalized_name > ")
                    .push_bind(c.n.clone())
                    .push(") OR (is_star::int = ")
                    .push_bind(p_star)
                    .push(" AND trending_score = ")
                    .push_bind(p_trend)
                    .push(" AND normalized_name = ")
                    .push_bind(c.n.clone())
                    .push(" AND id > ")
                    .push_bind(c.id)
                    .push("))");
            }
            "az" => {
                qb.push(" AND ((normalized_name > ")
                    .push_bind(c.n.clone())
                    .push(") OR (normalized_name = ")
                    .push_bind(c.n.clone())
                    .push(" AND id > ")
                    .push_bind(c.id)
                    .push("))");
            }
            "trending" => {
                let p = c.p as f32;
                qb.push(" AND (trending_score < ")
                    .push_bind(p)
                    .push(" OR (trending_score = ")
                    .push_bind(p)
                    .push(" AND normalized_name > ")
                    .push_bind(c.n.clone())
                    .push(") OR (trending_score = ")
                    .push_bind(p)
                    .push(" AND normalized_name = ")
                    .push_bind(c.n.clone())
                    .push(" AND id > ")
                    .push_bind(c.id)
                    .push("))");
            }
            _ => {
                let p = c.p as f32;
                qb.push(" AND (popularity_score < ")
                    .push_bind(p)
                    .push(" OR (popularity_score = ")
                    .push_bind(p)
                    .push(" AND normalized_name > ")
                    .push_bind(c.n.clone())
                    .push(") OR (popularity_score = ")
                    .push_bind(p)
                    .push(" AND normalized_name = ")
                    .push_bind(c.n.clone())
                    .push(" AND id > ")
                    .push_bind(c.id)
                    .push("))");
            }
        }
    }

    qb.push(" ORDER BY ")
        .push(order_clause)
        .push(" LIMIT ")
        .push_bind(limit);

    Ok(qb.build_query_as::<ArtistRow>().fetch_all(pg).await?)
}

async fn fetch_albums(
    pg: &PgPool,
    sort: &str,
    kind: Option<&str>,
    search: Option<&str>,
    cursor: Option<&AlbumCursor>,
    limit: i64,
) -> AppResult<Vec<AlbumRow>> {
    let order_clause = match sort {
        "popular" => "al.popularity_score DESC, al.normalized_title ASC, al.id ASC",
        "tracks" => "al.track_count DESC, al.normalized_title ASC, al.id ASC",
        "az" => "al.normalized_title ASC, al.id ASC",
        _ => {
            "COALESCE(al.release_date, make_date(COALESCE(al.release_year::int, 1970), 1, 1)) DESC, al.normalized_title ASC, al.id ASC"
        }
    };

    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "SELECT al.id, al.title, al.normalized_title, al.type AS kind, al.release_year, \
                al.release_date, al.cover_url, al.confidence, \
                al.track_count, al.total_duration_ms, al.popularity_score, al.is_star_artist, \
                al.primary_artist_id, \
                a.name AS primary_artist_name, \
                a.avatar_url AS primary_artist_avatar \
         FROM albums al \
         LEFT JOIN artists a ON a.id = al.primary_artist_id AND a.merged_into IS NULL \
         WHERE al.track_count > 0 \
           AND al.popularity_score > 0 \
           AND al.primary_artist_id IS NOT NULL \
           AND (al.release_year IS NULL \
                OR al.release_year <= EXTRACT(YEAR FROM CURRENT_DATE)::smallint)",
    );

    if let Some(k) = kind {
        qb.push(" AND al.type = ").push_bind(k.to_string());
    }

    if let Some(q) = search {
        let needle = format!("%{}%", q.trim().to_lowercase());
        qb.push(" AND (al.normalized_title LIKE ")
            .push_bind(needle.clone())
            .push(" OR LOWER(al.title) LIKE ")
            .push_bind(needle.clone())
            .push(" OR LOWER(COALESCE(a.name, '')) LIKE ")
            .push_bind(needle)
            .push(")");
    }

    if let Some(c) = cursor {
        match sort {
            "popular" => {
                let p = c.p as f32;
                qb.push(" AND (al.popularity_score < ")
                    .push_bind(p)
                    .push(" OR (al.popularity_score = ")
                    .push_bind(p)
                    .push(" AND al.normalized_title > ")
                    .push_bind(c.n.clone())
                    .push(") OR (al.popularity_score = ")
                    .push_bind(p)
                    .push(" AND al.normalized_title = ")
                    .push_bind(c.n.clone())
                    .push(" AND al.id > ")
                    .push_bind(c.id)
                    .push("))");
            }
            "tracks" => {
                let p = c.p as i32;
                qb.push(" AND (al.track_count < ")
                    .push_bind(p)
                    .push(" OR (al.track_count = ")
                    .push_bind(p)
                    .push(" AND al.normalized_title > ")
                    .push_bind(c.n.clone())
                    .push(") OR (al.track_count = ")
                    .push_bind(p)
                    .push(" AND al.normalized_title = ")
                    .push_bind(c.n.clone())
                    .push(" AND al.id > ")
                    .push_bind(c.id)
                    .push("))");
            }
            "az" => {
                qb.push(" AND ((al.normalized_title > ")
                    .push_bind(c.n.clone())
                    .push(") OR (al.normalized_title = ")
                    .push_bind(c.n.clone())
                    .push(" AND al.id > ")
                    .push_bind(c.id)
                    .push("))");
            }
            _ => {
                let cursor_date = NaiveDate::from_ymd_opt(1970, 1, 1)
                    .expect("static date 1970-01-01")
                    .checked_add_signed(chrono::Duration::days(c.p2 as i64))
                    .unwrap_or_else(|| {
                        NaiveDate::from_ymd_opt(1970, 1, 1).expect("static date 1970-01-01")
                    });
                let date_expr = "COALESCE(al.release_date, make_date(COALESCE(al.release_year::int, 1970), 1, 1))";
                qb.push(" AND (")
                    .push(date_expr)
                    .push(" < ")
                    .push_bind(cursor_date)
                    .push(" OR (")
                    .push(date_expr)
                    .push(" = ")
                    .push_bind(cursor_date)
                    .push(" AND al.normalized_title > ")
                    .push_bind(c.n.clone())
                    .push(") OR (")
                    .push(date_expr)
                    .push(" = ")
                    .push_bind(cursor_date)
                    .push(" AND al.normalized_title = ")
                    .push_bind(c.n.clone())
                    .push(" AND al.id > ")
                    .push_bind(c.id)
                    .push("))");
            }
        }
    }

    qb.push(" ORDER BY ")
        .push(order_clause)
        .push(" LIMIT ")
        .push_bind(limit);

    Ok(qb.build_query_as::<AlbumRow>().fetch_all(pg).await?)
}

async fn artists(
    State(st): State<AppState>,
    _: SessionCtx,
    Query(q): Query<ArtistsQuery>,
) -> AppResult<Json<ListResponse<CatalogArtist>>> {
    let limit = resolved_limit(q.limit);
    let sort = artist_sort_kind(q.sort.as_deref());
    let cursor: Option<ArtistCursor> = match q.cursor.as_deref() {
        Some(s) if !s.is_empty() => Some(cursor::decode(s)?),
        _ => None,
    };
    let rows = fetch_artists(
        &st.pg,
        sort,
        q.tag.as_deref().filter(|s| !s.is_empty()),
        q.q.as_deref().and_then(usable_search),
        cursor.as_ref(),
        limit + 1,
    )
    .await?;

    let next_cursor = cursor_row(rows.len(), limit)
        .and_then(|at| rows.get(at))
        .map(|r| cursor::encode(&artist_cursor_for_sort(sort, r)));

    let items: Vec<CatalogArtist> = rows
        .into_iter()
        .take(limit as usize)
        .map(|r| CatalogArtist {
            id: r.id,
            name: r.name,
            country: r.country,
            avatar_url: r.avatar_url,
            confidence: r.confidence,
            track_count_primary: r.track_count_primary,
            track_count_featured: r.track_count_featured,
            album_count: r.album_count_denorm,
            monthly_listeners: r.monthly_listeners,
            trending: r.trending_score,
            popularity: r.popularity_score,
            tags: canonicalize_tags(r.tags),
            star: r.is_star,
            aura_id: if r.is_star { r.star_aura_id } else { None },
            custom_hex: if r.is_star { r.star_custom_hex } else { None },
        })
        .collect();

    Ok(Json(ListResponse { items, next_cursor }))
}

async fn albums(
    State(st): State<AppState>,
    _: SessionCtx,
    Query(q): Query<AlbumsQuery>,
) -> AppResult<Json<ListResponse<CatalogAlbum>>> {
    let limit = resolved_limit(q.limit);
    let sort = album_sort_kind(q.sort.as_deref());
    let kind = album_kind_filter(q.kind.as_deref());
    let cursor: Option<AlbumCursor> = match q.cursor.as_deref() {
        Some(s) if !s.is_empty() => Some(cursor::decode(s)?),
        _ => None,
    };
    let rows = fetch_albums(
        &st.pg,
        sort,
        kind,
        q.q.as_deref().and_then(usable_search),
        cursor.as_ref(),
        limit + 1,
    )
    .await?;

    let next_cursor = cursor_row(rows.len(), limit)
        .and_then(|at| rows.get(at))
        .map(|r| cursor::encode(&album_cursor_for_sort(sort, r)));

    let items: Vec<CatalogAlbum> = rows
        .into_iter()
        .take(limit as usize)
        .map(map_album_row)
        .collect();

    Ok(Json(ListResponse { items, next_cursor }))
}

fn map_album_row(r: AlbumRow) -> CatalogAlbum {
    let release_month = r
        .release_date
        .map(|d| d.format("%-m").to_string().parse::<i32>().unwrap_or(0));
    let primary_artist = CatalogAlbumArtist {
        id: r.primary_artist_id.unwrap_or_else(Uuid::nil),
        name: r.primary_artist_name.unwrap_or_default(),
        avatar_url: r.primary_artist_avatar,
    };
    CatalogAlbum {
        id: r.id,
        title: r.title,
        kind: r.kind,
        release_year: r.release_year,
        release_month,
        cover_url: r.cover_url,
        confidence: r.confidence,
        primary_artist,
        track_count: r.track_count,
        total_duration_ms: r.total_duration_ms,
        popularity: r.popularity_score,
        star: r.is_star_artist,
    }
}

async fn albums_by_year(
    State(st): State<AppState>,
    _: SessionCtx,
    Query(q): Query<AlbumsByYearQuery>,
) -> AppResult<Json<YearBucketsResponse>> {
    let years = q.years.unwrap_or(8).clamp(1, 20);
    let per_year = q.per_year.unwrap_or(20).clamp(1, 40);
    let kind = album_kind_filter(q.kind.as_deref());

    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(
        "WITH max_y AS ( \
             SELECT LEAST( \
                 MAX(release_year), \
                 EXTRACT(YEAR FROM CURRENT_DATE)::smallint \
             ) AS year \
             FROM albums \
             WHERE track_count > 0 AND release_year IS NOT NULL \
               AND popularity_score > 0 AND primary_artist_id IS NOT NULL",
    );
    if let Some(k) = kind {
        qb.push(" AND type = ").push_bind(k.to_string());
    }
    qb.push(
        " ), years AS ( \
             SELECT generate_series( \
                 COALESCE((SELECT year FROM max_y), EXTRACT(YEAR FROM CURRENT_DATE)::int), \
                 COALESCE((SELECT year FROM max_y), EXTRACT(YEAR FROM CURRENT_DATE)::int) - (",
    );
    qb.push_bind(years as i32).push(
        "::int - 1), -1 \
             )::smallint AS year \
         ) \
         SELECT y.year AS bucket_year, \
                al.id, al.title, al.normalized_title, al.type AS kind, \
                al.release_year, al.release_date, al.cover_url, al.confidence, \
                al.track_count, al.total_duration_ms, al.popularity_score, al.is_star_artist, \
                al.primary_artist_id, \
                a.name AS primary_artist_name, \
                a.avatar_url AS primary_artist_avatar \
         FROM years y \
         CROSS JOIN LATERAL ( \
             SELECT * FROM albums al_inner \
             WHERE al_inner.release_year = y.year AND al_inner.track_count > 0 \
               AND al_inner.popularity_score > 0 \
               AND al_inner.primary_artist_id IS NOT NULL",
    );
    if let Some(k) = kind {
        qb.push(" AND al_inner.type = ").push_bind(k.to_string());
    }
    qb.push(
        " ORDER BY al_inner.popularity_score DESC, \
                   al_inner.release_date DESC NULLS LAST, \
                   al_inner.normalized_title ASC, \
                   al_inner.id ASC \
          LIMIT ",
    )
    .push_bind(per_year as i32)
    .push(
        " ) al \
         LEFT JOIN artists a ON a.id = al.primary_artist_id AND a.merged_into IS NULL \
         ORDER BY y.year DESC, al.popularity_score DESC, al.normalized_title ASC",
    );

    #[derive(sqlx::FromRow)]
    struct YearRow {
        bucket_year: i16,
        #[sqlx(flatten)]
        album: AlbumRow,
    }
    let rows = qb.build_query_as::<YearRow>().fetch_all(&st.pg).await?;

    let mut buckets: Vec<YearBucket> = Vec::new();
    let mut current_year: Option<i32> = None;
    for row in rows {
        let year = row.bucket_year as i32;
        if current_year != Some(year) {
            buckets.push(YearBucket {
                year,
                items: Vec::new(),
            });
            current_year = Some(year);
        }
        if let Some(b) = buckets.last_mut() {
            b.items.push(map_album_row(row.album));
        }
    }
    buckets.retain(|b| !b.items.is_empty());

    Ok(Json(YearBucketsResponse { buckets }))
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SpotlightItem {
    Artist { artist: CatalogArtist },
    Album { album: CatalogAlbum },
}

#[derive(Debug, Serialize)]
struct SpotlightResponse {
    items: Vec<SpotlightItem>,
}

#[derive(Debug, sqlx::FromRow)]
struct SettingsRow {
    show_star: bool,
    star_strategy: String,
    star_limit: i32,
}

async fn load_settings(pg: &PgPool) -> AppResult<SettingsRow> {
    let row = sqlx::query_file_as!(SettingsRow, "queries/discover/handlers/load_settings.sql")
        .fetch_optional(pg)
        .await?;
    Ok(row.unwrap_or(SettingsRow {
        show_star: true,
        star_strategy: "popular".into(),
        star_limit: 8,
    }))
}

async fn fetch_artists_by_ids(pg: &PgPool, ids: &[Uuid]) -> AppResult<Vec<ArtistRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_file_as!(
        ArtistRow,
        "queries/discover/handlers/artists_by_ids.sql",
        ids
    )
    .fetch_all(pg)
    .await?;
    Ok(rows)
}

async fn fetch_albums_by_ids(pg: &PgPool, ids: &[Uuid]) -> AppResult<Vec<AlbumRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_file_as!(AlbumRow, "queries/discover/handlers/albums_by_ids.sql", ids)
        .fetch_all(pg)
        .await?;
    Ok(rows)
}

async fn fetch_star_artists(
    pg: &PgPool,
    strategy: &str,
    limit: i64,
    exclude: &[Uuid],
) -> AppResult<Vec<ArtistRow>> {
    if limit <= 0 {
        return Ok(Vec::new());
    }

    if strategy == "random" {
        let rows = sqlx::query_file_as!(
            ArtistRow,
            "queries/discover/handlers/star_artists_random_sample.sql",
            exclude,
            limit
        )
        .fetch_all(pg)
        .await?;
        if !rows.is_empty() {
            return Ok(rows);
        }
        let rows = sqlx::query_file_as!(
            ArtistRow,
            "queries/discover/handlers/star_artists_random_fallback.sql",
            exclude,
            limit
        )
        .fetch_all(pg)
        .await?;
        return Ok(rows);
    }

    let rows = sqlx::query_file_as!(
        ArtistRow,
        "queries/discover/handlers/star_artists_popular.sql",
        exclude,
        limit
    )
    .fetch_all(pg)
    .await?;
    Ok(rows)
}

fn artist_row_to_catalog(r: ArtistRow) -> CatalogArtist {
    CatalogArtist {
        id: r.id,
        name: r.name,
        country: r.country,
        avatar_url: r.avatar_url,
        confidence: r.confidence,
        track_count_primary: r.track_count_primary,
        track_count_featured: r.track_count_featured,
        album_count: r.album_count_denorm,
        monthly_listeners: r.monthly_listeners,
        trending: r.trending_score,
        popularity: r.popularity_score,
        tags: canonicalize_tags(r.tags),
        star: r.is_star,
        aura_id: if r.is_star { r.star_aura_id } else { None },
        custom_hex: if r.is_star { r.star_custom_hex } else { None },
    }
}

async fn spotlight(
    State(st): State<AppState>,
    _: SessionCtx,
    Query(q): Query<SpotlightQuery>,
) -> AppResult<Json<SpotlightResponse>> {
    let settings = load_settings(&st.pg).await?;
    let requested = q.limit.unwrap_or(settings.star_limit as i64).clamp(0, 24);
    if requested == 0 {
        return Ok(Json(SpotlightResponse { items: Vec::new() }));
    }

    let promoted = sqlx::query_file!(
        "queries/discover/handlers/spotlight_promoted.sql",
        requested
    )
    .fetch_all(&st.pg)
    .await?;

    let mut artist_ids: Vec<Uuid> = Vec::new();
    let mut album_ids: Vec<Uuid> = Vec::new();
    for p in &promoted {
        match p.entity_type.as_str() {
            "artist" => artist_ids.push(p.entity_id),
            "album" => album_ids.push(p.entity_id),
            _ => {}
        }
    }

    let (artist_rows, album_rows) = tokio::try_join!(
        fetch_artists_by_ids(&st.pg, &artist_ids),
        fetch_albums_by_ids(&st.pg, &album_ids),
    )?;

    let artist_map: std::collections::HashMap<Uuid, ArtistRow> =
        artist_rows.into_iter().map(|r| (r.id, r)).collect();
    let album_map: std::collections::HashMap<Uuid, AlbumRow> =
        album_rows.into_iter().map(|r| (r.id, r)).collect();

    let mut items: Vec<SpotlightItem> = Vec::with_capacity(requested as usize);
    let mut used_artist_ids: Vec<Uuid> = Vec::new();

    for p in promoted {
        match p.entity_type.as_str() {
            "artist" => {
                if let Some(row) = artist_map.get(&p.entity_id) {
                    used_artist_ids.push(row.id);
                    items.push(SpotlightItem::Artist {
                        artist: artist_row_to_catalog(row.clone()),
                    });
                }
            }
            "album" => {
                if let Some(row) = album_map.get(&p.entity_id) {
                    items.push(SpotlightItem::Album {
                        album: map_album_row(row.clone()),
                    });
                }
            }
            _ => {}
        }
        if items.len() as i64 >= requested {
            break;
        }
    }

    let remaining = requested - items.len() as i64;
    if settings.show_star && remaining > 0 {
        let star_rows =
            fetch_star_artists(&st.pg, &settings.star_strategy, remaining, &used_artist_ids)
                .await?;
        for r in star_rows {
            items.push(SpotlightItem::Artist {
                artist: artist_row_to_catalog(r),
            });
        }
    }

    Ok(Json(SpotlightResponse { items }))
}

async fn summary(State(st): State<AppState>, _: SessionCtx) -> AppResult<Json<DiscoverSummary>> {
    let s = st.discover.cached_summary().await?;
    Ok(Json(DiscoverSummary {
        artists_count: s.artists_count,
        albums_count: s.albums_count,
        fresh_count: s.fresh_count,
        fresh_window_days: s.fresh_window_days as i64,
    }))
}

#[derive(Debug, Serialize)]
struct RandomResponse {
    id: Uuid,
}

#[derive(Debug, Deserialize)]
struct TagsQuery {
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct CatalogTag {
    id: String,
    label: String,
    count: i64,
}

async fn tags(
    State(st): State<AppState>,
    _: SessionCtx,
    Query(q): Query<TagsQuery>,
) -> AppResult<Json<ListResponse<CatalogTag>>> {
    let limit = q.limit.unwrap_or(12).clamp(1, 64) as usize;

    let cached: CachedTagList = st.discover.cached_tag_list().await?;

    let items: Vec<CatalogTag> = cached
        .items
        .into_iter()
        .take(limit)
        .filter_map(|tg| {
            canonicalize_tag(&tg.id).map(|label| CatalogTag {
                id: tg.id,
                label,
                count: tg.count,
            })
        })
        .collect();

    Ok(Json(ListResponse {
        items,
        next_cursor: None,
    }))
}

async fn random(
    State(st): State<AppState>,
    _: SessionCtx,
    Query(q): Query<RandomQuery>,
) -> AppResult<Json<RandomResponse>> {
    let target = q.kind.as_deref().unwrap_or("album");
    let id = match target {
        "artist" => pick_random_artist(&st.pg).await?,
        _ => pick_random_album(&st.pg).await?,
    };
    let id = id.ok_or_else(|| AppError::not_found("catalog empty"))?;
    Ok(Json(RandomResponse { id }))
}

async fn pick_random_album(pg: &PgPool) -> AppResult<Option<Uuid>> {
    let row = sqlx::query_file_scalar!("queries/discover/handlers/pick_random_album_sample.sql")
        .fetch_optional(pg)
        .await?;
    if let Some(id) = row {
        return Ok(Some(id));
    }
    let row = sqlx::query_file_scalar!("queries/discover/handlers/pick_random_album_fallback.sql")
        .fetch_optional(pg)
        .await?;
    Ok(row)
}

async fn pick_random_artist(pg: &PgPool) -> AppResult<Option<Uuid>> {
    let row = sqlx::query_file_scalar!("queries/discover/handlers/pick_random_artist_sample.sql")
        .fetch_optional(pg)
        .await?;
    if let Some(id) = row {
        return Ok(Some(id));
    }
    let row = sqlx::query_file_scalar!("queries/discover/handlers/pick_random_artist_fallback.sql")
        .fetch_optional(pg)
        .await?;
    Ok(row)
}
