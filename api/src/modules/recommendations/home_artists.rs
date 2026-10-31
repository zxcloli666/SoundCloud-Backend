use std::collections::HashSet;

use deadpool_redis::redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use super::clusters::ClusterNeighbor;
use super::service::RecommendationsService;
use super::service::util::user_id_variants;

const ARTISTS_TTL_SECS: u64 = 600;
const ARTISTS_CACHE: &str = "home_artists";

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct HomeArtist {
    pub artist_id: Uuid,
    pub artist_name: String,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Copy)]
pub(crate) enum Shelf {
    Top,
    Adjacent,
}

impl Shelf {
    fn name(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Adjacent => "adjacent",
        }
    }
}

pub(crate) async fn load_artists(
    pg: &PgPool,
    shelf: Shelf,
    variants: &[String],
    limit: i64,
) -> Result<Vec<HomeArtist>, sqlx::Error> {
    match shelf {
        Shelf::Top => {
            sqlx::query_file_as!(
                HomeArtist,
                "queries/recommendations/home_wave/top_artists.sql",
                variants,
                limit
            )
            .fetch_all(pg)
            .await
        }
        Shelf::Adjacent => {
            sqlx::query_file_as!(
                HomeArtist,
                "queries/recommendations/home_wave/adjacent_artists.sql",
                variants,
                limit
            )
            .fetch_all(pg)
            .await
        }
    }
}

fn cache_key(shelf: Shelf, sc_user_id: &str, limit: i64) -> String {
    format!("home:artists:{}:{limit}:{sc_user_id}", shelf.name())
}

impl RecommendationsService {
    pub(super) async fn load_top_artists_cluster(
        &self,
        sc_user_id: &str,
        exclude: &HashSet<String>,
        limit: i64,
    ) -> Vec<ClusterNeighbor> {
        self.shelf_cluster(Shelf::Top, sc_user_id, exclude, limit)
            .await
    }

    pub(super) async fn load_adjacent_artists_cluster(
        &self,
        sc_user_id: &str,
        exclude: &HashSet<String>,
        limit: i64,
    ) -> Vec<ClusterNeighbor> {
        self.shelf_cluster(Shelf::Adjacent, sc_user_id, exclude, limit)
            .await
    }

    async fn shelf_cluster(
        &self,
        shelf: Shelf,
        sc_user_id: &str,
        exclude: &HashSet<String>,
        limit: i64,
    ) -> Vec<ClusterNeighbor> {
        let Some(artists) = self.shelf_artists(shelf, sc_user_id, limit).await else {
            return Vec::new();
        };
        let ids: Vec<Uuid> = artists.iter().map(|artist| artist.artist_id).collect();
        let best = self.home_tracks.best_track(&self.pg, &ids, exclude).await;
        artists
            .into_iter()
            .filter_map(|artist| {
                let track = best.get(&artist.artist_id)?;
                Some(ClusterNeighbor {
                    track_id: track.to_string(),
                    artist_id: artist.artist_id,
                    artist_name: artist.artist_name,
                    avatar_url: artist.avatar_url,
                })
            })
            .collect()
    }

    async fn shelf_artists(
        &self,
        shelf: Shelf,
        sc_user_id: &str,
        limit: i64,
    ) -> Option<Vec<HomeArtist>> {
        let key = cache_key(shelf, sc_user_id, limit);
        if let Some(cached) = self.read_artists(&key).await {
            crate::metrics::record_cache(ARTISTS_CACHE, true);
            return Some(cached);
        }
        crate::metrics::record_cache(ARTISTS_CACHE, false);
        let variants = user_id_variants(sc_user_id);
        let artists = load_artists(&self.pg, shelf, &variants, limit).await.ok()?;
        if !artists.is_empty() {
            self.write_artists(&key, &artists).await;
        }
        Some(artists)
    }

    async fn read_artists(&self, key: &str) -> Option<Vec<HomeArtist>> {
        let mut conn = self.redis.get().await.ok()?;
        let raw: Option<String> = conn.get(key).await.ok().flatten();
        serde_json::from_str(&raw?).ok()
    }

    async fn write_artists(&self, key: &str, artists: &[HomeArtist]) {
        let Ok(payload) = serde_json::to_string(artists) else {
            return;
        };
        let Ok(mut conn) = self.redis.get().await else {
            return;
        };
        let _: Result<(), _> = conn
            .set_ex::<_, _, ()>(key, payload, ARTISTS_TTL_SECS)
            .await;
    }
}
