use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::NaiveDate;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use tokio::sync::Semaphore;
use uuid::Uuid;

use super::failure::{self, Shed, search_busy, search_timeout};
use super::terms::{self, Shape};
use crate::cache::cache_service::CacheScope;
use crate::cache::{CacheService, KeyedCoalesce, ListPageResult, build_list_cache_key};
use crate::error::{AppError, AppResult};
use crate::modules::enrich::dto as enrich_dto;
use crate::modules::playlists::{PlaylistRow, project_to_sc_shape as project_playlist};
use crate::modules::tracks::repository::project_many_public;
use crate::modules::users::{UserRow, project_to_sc_shape as project_user};
use catalog_normalize::normalize_name;

const TTL_SECONDS: u64 = 60;
const MIN_QUERY_LEN: usize = 2;
const MAX_QUERY_LEN: usize = 128;
pub const MAX_PAGE: i64 = 24;
pub const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 50;
const PERMITS: usize = 8;
const PERMIT_WAIT: Duration = Duration::from_secs(1);
const BEGIN_WAIT: Duration = Duration::from_secs(1);
const BUDGET: Duration = Duration::from_millis(2500);

pub struct SearchService {
    pub(super) pg: PgPool,
    cache: Arc<CacheService>,
    flights: KeyedCoalesce<Result<String, Shed>>,
    pub(super) permits: Semaphore,
}

#[derive(Debug, Clone)]
pub(super) struct Request {
    pub(super) q: Option<String>,
    pub(super) page: i64,
    pub(super) limit: i64,
}

impl Request {
    pub(super) fn new(raw: &str, page: i64, limit: i64) -> Self {
        let cut: String = raw.trim().chars().take(MAX_QUERY_LEN).collect();
        let q = normalize_name(&cut);
        Self {
            q: (q.chars().count() >= MIN_QUERY_LEN).then_some(q),
            page: page.clamp(0, MAX_PAGE),
            limit: limit.clamp(1, MAX_LIMIT),
        }
    }

    pub(super) fn fetch(&self) -> i64 {
        self.limit + 1
    }

    pub(super) fn offset(&self) -> i64 {
        self.page * self.limit
    }

    pub(super) fn cut<T>(&self, mut rows: Vec<T>) -> (Vec<T>, bool) {
        let more = rows.len() as i64 > self.limit;
        rows.truncate(self.limit as usize);
        (rows, more && self.page < MAX_PAGE)
    }

    pub(super) fn key(&self, prefix: &str, q: &str, owner: Option<&str>) -> String {
        build_list_cache_key(
            prefix,
            &[
                ("q", q.to_owned()),
                ("owner", owner.unwrap_or_default().to_owned()),
                ("page", self.page.to_string()),
                ("limit", self.limit.to_string()),
            ],
        )
    }

    fn page(&self, collection: Vec<Value>, has_more: bool) -> ListPageResult<Value> {
        ListPageResult {
            collection,
            page: self.page,
            page_size: self.limit,
            has_more,
        }
    }

    fn empty(&self) -> ListPageResult<Value> {
        self.page(Vec::new(), false)
    }
}

fn parse_owner(raw: Option<&str>) -> Result<Option<String>, ()> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(None);
    };
    let bare = raw.strip_prefix("soundcloud:users:").unwrap_or(raw);
    if !bare.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(());
    }
    match bare.parse::<u64>() {
        Ok(id) if id > 0 => Ok(Some(id.to_string())),
        _ => Err(()),
    }
}

impl SearchService {
    pub fn new(pg: PgPool, cache: Arc<CacheService>) -> Arc<Self> {
        Arc::new(Self {
            pg,
            cache,
            flights: KeyedCoalesce::new(),
            permits: Semaphore::new(PERMITS),
        })
    }

    pub(super) async fn cached<T, F, Fut>(&self, key: &str, compute: F) -> AppResult<T>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: Future<Output = AppResult<T>>,
    {
        if let Ok(Some(raw)) = self.cache.get_raw(key).await
            && let Ok(value) = serde_json::from_str::<T>(&raw)
        {
            return Ok(value);
        }
        let json = self
            .flights
            .run(key, || async {
                let value = match self.guarded(compute).await {
                    Ok(value) => value,
                    Err(error) => return Shed::of(&error).map(Err).ok_or(error),
                };
                let json = serde_json::to_string(&value)
                    .map_err(|error| AppError::internal(error.to_string()))?;
                let _ = self
                    .cache
                    .set_raw(key, &json, TTL_SECONDS, None, CacheScope::Shared, None)
                    .await;
                Ok::<_, AppError>(Ok(json))
            })
            .await?
            .map_err(Shed::error)?;
        serde_json::from_str::<T>(&json).map_err(|error| AppError::internal(error.to_string()))
    }

    async fn guarded<T, F, Fut>(&self, compute: F) -> AppResult<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = AppResult<T>>,
    {
        let run = async {
            let _permit = tokio::time::timeout(PERMIT_WAIT, self.permits.acquire())
                .await
                .map_err(|_| search_busy())?
                .map_err(|_| search_busy())?;
            compute().await
        };
        match tokio::time::timeout(BUDGET, run).await {
            Ok(result) => result.map_err(failure::map),
            Err(_) => Err(search_timeout()),
        }
    }

    pub(super) async fn begin(&self) -> AppResult<Transaction<'static, Postgres>> {
        let mut tx = tokio::time::timeout(BEGIN_WAIT, self.pg.begin())
            .await
            .map_err(|_| search_busy())??;
        sqlx::query_file!("queries/search/configure.sql")
            .fetch_one(&mut *tx)
            .await?;
        Ok(tx)
    }

    pub async fn tracks(
        &self,
        q: &str,
        user_urn: Option<&str>,
        page: i64,
        limit: i64,
    ) -> AppResult<ListPageResult<Value>> {
        let request = Request::new(q, page, limit);
        let (Some(q), Ok(owner)) = (request.q.clone(), parse_owner(user_urn)) else {
            return Ok(request.empty());
        };
        let key = request.key("search-db-v2-tracks", &q, owner.as_deref());
        self.cached(&key, || async {
            let mut tx = self.begin().await?;
            let Some(terms) = terms::resolve(&mut tx, &q, Shape::Tracks).await? else {
                return Ok(request.empty());
            };
            let ids = sqlx::query_file_scalar!(
                "queries/search/tracks.sql",
                terms.strict,
                terms.loose,
                q,
                owner,
                request.fetch(),
                request.offset(),
                terms.variant
            )
            .fetch_all(&mut *tx)
            .await?;
            tx.commit().await?;
            let (ids, more) = request.cut(ids);
            let mut collection: Vec<Value> = project_many_public(&self.pg, &ids)
                .await?
                .into_iter()
                .flatten()
                .collect();
            enrich_dto::apply_to_tracks(&self.pg, &mut collection).await?;
            Ok(request.page(collection, more))
        })
        .await
    }

    pub async fn playlists(
        &self,
        q: &str,
        user_urn: Option<&str>,
        page: i64,
        limit: i64,
    ) -> AppResult<ListPageResult<Value>> {
        let request = Request::new(q, page, limit);
        let (Some(q), Ok(owner)) = (request.q.clone(), parse_owner(user_urn)) else {
            return Ok(request.empty());
        };
        let key = request.key("search-db-v2-playlists", &q, owner.as_deref());
        self.cached(&key, || async {
            let mut tx = self.begin().await?;
            let Some(terms) = terms::resolve(&mut tx, &q, Shape::Entities).await? else {
                return Ok(request.empty());
            };
            let rows = sqlx::query_file_as!(
                PlaylistRow,
                "queries/search/playlists.sql",
                terms.strict,
                terms.loose,
                q,
                owner,
                request.fetch(),
                request.offset()
            )
            .fetch_all(&mut *tx)
            .await?;
            tx.commit().await?;
            let (rows, more) = request.cut(rows);
            let collection = project_playlists_with_owners(&self.pg, rows).await?;
            Ok(request.page(collection, more))
        })
        .await
    }

    pub async fn users(&self, q: &str, page: i64, limit: i64) -> AppResult<ListPageResult<Value>> {
        let request = Request::new(q, page, limit);
        let Some(q) = request.q.clone() else {
            return Ok(request.empty());
        };
        let key = request.key("search-db-v2-users", &q, None);
        self.cached(&key, || async {
            let mut tx = self.begin().await?;
            let Some(terms) = terms::resolve(&mut tx, &q, Shape::Entities).await? else {
                return Ok(request.empty());
            };
            let rows = sqlx::query_file_as!(
                UserRow,
                "queries/search/users.sql",
                terms.strict,
                terms.loose,
                q,
                request.fetch(),
                request.offset()
            )
            .fetch_all(&mut *tx)
            .await?;
            tx.commit().await?;
            let (rows, more) = request.cut(rows);
            Ok(request.page(rows.iter().map(project_user).collect(), more))
        })
        .await
    }

    pub async fn artists(
        &self,
        q: &str,
        page: i64,
        limit: i64,
    ) -> AppResult<ListPageResult<Value>> {
        let request = Request::new(q, page, limit);
        let Some(q) = request.q.clone() else {
            return Ok(request.empty());
        };
        let key = request.key("search-db-v2-artists", &q, None);
        self.cached(&key, || async {
            let mut tx = self.begin().await?;
            let Some(terms) = terms::resolve(&mut tx, &q, Shape::Entities).await? else {
                return Ok(request.empty());
            };
            let rows = sqlx::query_file_as!(
                ArtistSearchRow,
                "queries/search/artists.sql",
                terms.strict,
                terms.loose,
                q,
                request.fetch(),
                request.offset()
            )
            .fetch_all(&mut *tx)
            .await?;
            tx.commit().await?;
            let (rows, more) = request.cut(rows);
            Ok(request.page(rows.into_iter().map(artist_to_value).collect(), more))
        })
        .await
    }

    pub async fn albums(&self, q: &str, page: i64, limit: i64) -> AppResult<ListPageResult<Value>> {
        let request = Request::new(q, page, limit);
        let Some(q) = request.q.clone() else {
            return Ok(request.empty());
        };
        let key = request.key("search-db-v2-albums", &q, None);
        self.cached(&key, || async {
            let mut tx = self.begin().await?;
            let Some(terms) = terms::resolve(&mut tx, &q, Shape::Entities).await? else {
                return Ok(request.empty());
            };
            let rows = sqlx::query_file_as!(
                AlbumSearchRow,
                "queries/search/albums.sql",
                terms.strict,
                terms.loose,
                q,
                request.fetch(),
                request.offset()
            )
            .fetch_all(&mut *tx)
            .await?;
            tx.commit().await?;
            let (rows, more) = request.cut(rows);
            Ok(request.page(rows.into_iter().map(album_to_value).collect(), more))
        })
        .await
    }
}

async fn project_playlists_with_owners(
    pg: &PgPool,
    rows: Vec<PlaylistRow>,
) -> AppResult<Vec<Value>> {
    let owner_ids: Vec<String> = rows
        .iter()
        .filter_map(|row| row.owner_sc_user_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let owners: HashMap<String, Value> = if owner_ids.is_empty() {
        HashMap::new()
    } else {
        sqlx::query_file_as!(UserRow, "queries/search/users_by_sc_ids.sql", &owner_ids)
            .fetch_all(pg)
            .await?
            .into_iter()
            .map(|user| (user.sc_user_id.clone(), project_user(&user)))
            .collect()
    };
    Ok(rows
        .iter()
        .map(|row| {
            let owner = row
                .owner_sc_user_id
                .as_deref()
                .and_then(|id| owners.get(id));
            project_playlist(row, owner)
        })
        .collect())
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

fn artist_to_value(r: ArtistSearchRow) -> Value {
    json!({
        "id": r.id,
        "name": r.name,
        "country": r.country,
        "avatar_url": r.avatar_url,
        "confidence": r.confidence,
        "track_count_primary": r.track_count_primary,
        "track_count_featured": r.track_count_featured,
        "album_count": r.album_count_denorm,
        "monthly_listeners": r.monthly_listeners,
        "trending": r.trending_score,
        "tags": crate::modules::discover::tags::canonicalize_tags(r.tags),
        "star": r.is_star,
        "aura_id": if r.is_star { r.star_aura_id } else { None },
        "custom_hex": if r.is_star { r.star_custom_hex } else { None },
    })
}

fn album_to_value(r: AlbumSearchRow) -> Value {
    let release_month = r
        .release_date
        .map(|d| d.format("%-m").to_string().parse::<i32>().unwrap_or(0));
    json!({
        "id": r.id,
        "title": r.title,
        "type": r.kind,
        "release_year": r.release_year,
        "release_month": release_month,
        "cover_url": r.cover_url,
        "confidence": r.confidence,
        "primary_artist": {
            "id": r.primary_artist_id.unwrap_or_else(Uuid::nil),
            "name": r.primary_artist_name.unwrap_or_default(),
            "avatar_url": r.primary_artist_avatar,
        },
        "track_count": r.track_count,
        "total_duration_ms": r.total_duration_ms,
        "popularity": r.popularity_score,
        "star": r.is_star_artist,
    })
}
