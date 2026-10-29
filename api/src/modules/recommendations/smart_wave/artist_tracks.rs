use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use mini_moka::sync::Cache;
use sqlx::PgPool;
use uuid::Uuid;

use super::graph;
use crate::cache::KeyedCoalesce;

const TTL: Duration = Duration::from_secs(600);
const KEPT: u64 = 20_000;
const TRACKS_KEPT: i64 = 64;
const RECHECK_SLACK: usize = 6;

pub struct Top {
    tracks: Vec<(u64, i64)>,
    complete: bool,
}

type Tops = Arc<HashMap<Uuid, Arc<Top>>>;

pub struct ArtistTopTracks {
    by_artist: Cache<Uuid, Arc<Top>>,
    flights: KeyedCoalesce<Tops>,
    loads: AtomicU64,
}

impl ArtistTopTracks {
    pub fn new() -> Self {
        Self {
            by_artist: Cache::builder()
                .max_capacity(KEPT)
                .time_to_live(TTL)
                .build(),
            flights: KeyedCoalesce::new(),
            loads: AtomicU64::new(0),
        }
    }

    #[cfg(test)]
    pub fn loads(&self) -> u64 {
        self.loads.load(Ordering::Relaxed)
    }

    pub async fn collect(
        &self,
        pg: &PgPool,
        artists: &[Uuid],
        exclude: &[String],
        per_artist: i64,
        total: i64,
    ) -> Vec<(u64, Uuid)> {
        if artists.is_empty() {
            return Vec::new();
        }
        let Some(tops) = self.tops(pg, artists).await else {
            return Vec::new();
        };
        let excluded: HashSet<u64> = exclude.iter().filter_map(|id| id.parse().ok()).collect();
        let per_artist = usize::try_from(per_artist).unwrap_or(0);
        let mut ordered: Vec<Uuid> = tops.keys().copied().collect();
        ordered.sort();

        let mut open: HashMap<Uuid, Vec<u64>> = HashMap::new();
        for artist in &ordered {
            let ids = tops[artist]
                .tracks
                .iter()
                .map(|(id, _)| *id)
                .filter(|id| !excluded.contains(id))
                .collect();
            open.insert(*artist, ids);
        }
        let checked: Vec<String> = open
            .values()
            .flat_map(|ids| ids.iter().take(per_artist + RECHECK_SLACK))
            .map(u64::to_string)
            .collect();
        let Some(visible) = visible_now(pg, &checked).await else {
            return Vec::new();
        };

        let mut picked: HashMap<Uuid, Vec<u64>> = HashMap::new();
        let mut short = Vec::new();
        for artist in &ordered {
            let ids = &open[artist];
            let examined = ids.len().min(per_artist + RECHECK_SLACK);
            let found: Vec<u64> = ids[..examined]
                .iter()
                .copied()
                .filter(|id| visible.contains(&id.to_string()))
                .take(per_artist)
                .collect();
            let unseen = ids.len() > examined || !tops[artist].complete;
            if found.len() < per_artist && unseen {
                short.push(*artist);
            } else {
                picked.insert(*artist, found);
            }
        }
        if !short.is_empty() {
            for (id, artist) in
                graph::collect_artist_tracks(pg, &short, exclude, per_artist as i64, total).await
            {
                picked.entry(artist).or_default().push(id);
            }
        }

        let mut rows = Vec::new();
        for artist in &ordered {
            for id in picked.remove(artist).unwrap_or_default() {
                rows.push((id, *artist));
            }
        }
        rows.truncate(usize::try_from(total).unwrap_or(0));
        rows
    }

    async fn tops(&self, pg: &PgPool, artists: &[Uuid]) -> Option<HashMap<Uuid, Arc<Top>>> {
        let mut found = HashMap::new();
        let mut missing = Vec::new();
        for artist in artists {
            match self.by_artist.get(artist) {
                Some(top) => {
                    crate::metrics::record_artist_tracks_cache(true);
                    found.insert(*artist, top);
                }
                None => {
                    crate::metrics::record_artist_tracks_cache(false);
                    missing.push(*artist);
                }
            }
        }
        if missing.is_empty() {
            return Some(found);
        }
        missing.sort();
        missing.dedup();
        let flight = missing
            .iter()
            .map(|artist| artist.simple().to_string())
            .collect::<Vec<_>>()
            .join(":");
        let loaded = self
            .flights
            .run(&flight, || async {
                if let Some(cached) = self.cached(&missing) {
                    return Ok(Arc::new(cached));
                }
                self.loads.fetch_add(1, Ordering::Relaxed);
                self.load(pg, &missing).await.map(Arc::new)
            })
            .await
            .ok()?;
        found.extend(loaded.iter().map(|(artist, top)| (*artist, top.clone())));
        Some(found)
    }

    fn cached(&self, artists: &[Uuid]) -> Option<HashMap<Uuid, Arc<Top>>> {
        artists
            .iter()
            .map(|artist| self.by_artist.get(artist).map(|top| (*artist, top)))
            .collect()
    }

    async fn load(
        &self,
        pg: &PgPool,
        artists: &[Uuid],
    ) -> Result<HashMap<Uuid, Arc<Top>>, sqlx::Error> {
        let rows = sqlx::query_file!(
            "queries/recommendations/smart_wave/graph/load_artist_top_tracks.sql",
            artists,
            TRACKS_KEPT
        )
        .fetch_all(pg)
        .await?;
        let mut grouped: HashMap<Uuid, (Vec<(u64, i64)>, bool)> = artists
            .iter()
            .map(|artist| (*artist, (Vec::new(), false)))
            .collect();
        for row in rows {
            let Ok(id) = row.sc_track_id.parse::<u64>() else {
                continue;
            };
            let entry = grouped.entry(row.artist_id).or_default();
            entry.0.push((id, row.play_count));
            entry.1 |= row.truncated;
        }
        let tops: HashMap<Uuid, Arc<Top>> = grouped
            .into_iter()
            .map(|(artist, (tracks, truncated))| {
                let top = Arc::new(Top {
                    tracks,
                    complete: !truncated,
                });
                (artist, top)
            })
            .collect();
        for (artist, top) in &tops {
            self.by_artist.insert(*artist, top.clone());
        }
        Ok(tops)
    }
}

impl Default for ArtistTopTracks {
    fn default() -> Self {
        Self::new()
    }
}

async fn visible_now(pg: &PgPool, ids: &[String]) -> Option<HashSet<String>> {
    if ids.is_empty() {
        return Some(HashSet::new());
    }
    sqlx::query_file_scalar!(
        "queries/recommendations/smart_wave/graph/visible_tracks.sql",
        ids
    )
    .fetch_all(pg)
    .await
    .ok()
    .map(|rows| rows.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: Uuid = Uuid::from_u128(1);
    const BIG: Uuid = Uuid::from_u128(2);
    const EMPTY: Uuid = Uuid::from_u128(3);
    const CALLERS: usize = 16;

    async fn artist(pool: &PgPool, id: Uuid, name: &str) -> anyhow::Result<()> {
        sqlx::query(
            "INSERT INTO artists (id, name, normalized_name, source) VALUES ($1, $2, $2, 'test')",
        )
        .bind(id)
        .bind(name)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn tracks(
        pool: &PgPool,
        artist: Uuid,
        first: i64,
        count: i64,
        hidden_every: i64,
    ) -> anyhow::Result<()> {
        sqlx::query(
            "WITH made AS (
                 INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms,
                                     sharing, storage_state, index_state)
                 SELECT n::text, 'urn:' || n, 'T' || n, 't' || n, 1000,
                        CASE WHEN $4 > 0 AND n % $4 = 0 THEN 'private' ELSE 'public' END,
                        'ok', 'indexed'
                 FROM generate_series($2::bigint, $2::bigint + $3::bigint - 1) AS n
                 RETURNING id, sc_track_id
             ), credited AS (
                 INSERT INTO track_artists (track_id, artist_id, role, source)
                 SELECT id, $1, 'primary', 'test' FROM made
             )
             INSERT INTO sc_track_counters (sc_track_id, play_count)
             SELECT sc_track_id, (sc_track_id::bigint * 7919) % 100003 FROM made",
        )
        .bind(artist)
        .bind(first)
        .bind(count)
        .bind(hidden_every)
        .execute(pool)
        .await?;
        Ok(())
    }

    async fn seed(pool: &PgPool) -> anyhow::Result<()> {
        artist(pool, SMALL, "small").await?;
        artist(pool, BIG, "big").await?;
        artist(pool, EMPTY, "empty").await?;
        tracks(pool, SMALL, 1000, 9, 4).await?;
        tracks(pool, BIG, 2000, 150, 5).await?;
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_cached_path_returns_what_the_query_returns(pool: PgPool) -> anyhow::Result<()> {
        seed(&pool).await?;
        let cache = ArtistTopTracks::new();
        let excluded: Vec<String> = ["2007", "2011", "1003", "2150"]
            .iter()
            .map(|id| (*id).to_owned())
            .collect();

        for artists in [
            vec![SMALL],
            vec![BIG],
            vec![BIG, SMALL, EMPTY],
            vec![EMPTY, SMALL],
        ] {
            for exclude in [&Vec::new(), &excluded] {
                for (per_artist, total) in [(6, 800), (3, 5), (12, 800), (6, 0)] {
                    for _ in 0..2 {
                        let cached = cache
                            .collect(&pool, &artists, exclude, per_artist, total)
                            .await;
                        let direct = graph::collect_artist_tracks(
                            &pool, &artists, exclude, per_artist, total,
                        )
                        .await;
                        assert_eq!(cached, direct, "{artists:?} {per_artist} {total}");
                    }
                }
            }
        }
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn sixteen_concurrent_misses_make_one_database_call(small: PgPool) -> anyhow::Result<()> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(40)
            .connect_with((*small.connect_options()).clone())
            .await?;
        seed(&pool).await?;
        let cache = Arc::new(ArtistTopTracks::new());

        let calls: Vec<_> = (0..CALLERS)
            .map(|_| {
                let pool = pool.clone();
                let cache = cache.clone();
                tokio::spawn(async move {
                    cache
                        .collect(&pool, &[SMALL, BIG, EMPTY], &[], 6, 800)
                        .await
                })
            })
            .collect();
        let mut results = Vec::new();
        for call in calls {
            results.push(call.await?);
        }

        assert_eq!(cache.loads(), 1);
        assert!(results.iter().all(|rows| rows == &results[0]));
        assert!(!results[0].is_empty());
        let before = cache.loads();
        cache
            .collect(&pool, &[SMALL, BIG, EMPTY], &[], 6, 800)
            .await;
        assert_eq!(cache.loads(), before);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_track_hidden_after_caching_is_not_served(pool: PgPool) -> anyhow::Result<()> {
        seed(&pool).await?;
        let cache = ArtistTopTracks::new();
        let artists = [BIG, SMALL];
        let first = cache.collect(&pool, &artists, &[], 6, 800).await;
        let top = first
            .iter()
            .find(|(_, a)| *a == BIG)
            .map(|(id, _)| *id)
            .unwrap();

        sqlx::query("UPDATE tracks SET sharing = 'private' WHERE sc_track_id = $1")
            .bind(top.to_string())
            .execute(&pool)
            .await?;
        let second = cache.collect(&pool, &artists, &[], 6, 800).await;
        assert_eq!(cache.loads(), 1);
        assert!(!second.iter().any(|(id, _)| *id == top));
        assert_eq!(
            second.iter().filter(|(_, a)| *a == BIG).count(),
            first.iter().filter(|(_, a)| *a == BIG).count()
        );

        sqlx::query("UPDATE tracks SET storage_state = 'pending'")
            .execute(&pool)
            .await?;
        assert!(cache.collect(&pool, &artists, &[], 6, 800).await.is_empty());
        Ok(())
    }
}
