use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use deadpool_redis::Pool as RedisPool;
use deadpool_redis::redis::AsyncCommands;
use mini_moka::sync::Cache;
use sqlx::PgPool;
use tokio::sync::Semaphore;
use tracing::debug;
use uuid::Uuid;

use crate::modules::recommendations::service::RecommendationsService;
use crate::modules::recommendations::service::util::user_id_variants;

const SEED_LIMIT: i64 = 48;
const LIKES_WINDOW_DAYS: i32 = 365;
const PLAYS_WINDOW_DAYS: i32 = 120;
const PLAY_BOOST: f32 = 0.6;
const HOPS: usize = 3;
const GAMMA: f32 = 0.9;
const EPS: f32 = 0.004;
const PATH_DECAY: f32 = 0.5;
const PROP_CAP: f32 = 0.98;
const FRONTIER_CAP: usize = 320;
const TOTAL_CAP: usize = 1500;
const DISLIKE_ARTIST_MIN: i64 = 3;
const ANTISPREAD_MU: f32 = 0.6;
const CACHE_TTL_SECS: u64 = 90;
const EDGES_TTL: Duration = Duration::from_secs(600);
const EDGES_KEPT: u64 = 4_000_000;

pub enum GraphSeed {
    User,
    Track(u64),
    Artist(Uuid),
}

pub type Affinity = HashMap<Uuid, f32>;
type Neighbors = HashMap<i16, Vec<(Uuid, f32)>>;
type Adjacency = HashMap<Uuid, Arc<Neighbors>>;

pub struct GraphEdges {
    by_artist: Cache<Uuid, Arc<Neighbors>>,
    loading: Mutex<HashMap<Uuid, Arc<Semaphore>>>,
}

struct Loading<'a> {
    edges: &'a GraphEdges,
    artists: Vec<(Uuid, Arc<Semaphore>)>,
}

impl Drop for Loading<'_> {
    fn drop(&mut self) {
        let mut loading = self
            .edges
            .loading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (artist, done) in &self.artists {
            loading.remove(artist);
            done.close();
        }
    }
}

impl GraphEdges {
    pub fn new() -> Self {
        Self {
            by_artist: Cache::builder()
                .max_capacity(EDGES_KEPT)
                .weigher(|_, neighbors: &Arc<Neighbors>| edge_count(neighbors))
                .time_to_live(EDGES_TTL)
                .build(),
            loading: Mutex::new(HashMap::new()),
        }
    }

    async fn of(&self, pg: &PgPool, artists: &[Uuid]) -> Adjacency {
        let mut adjacency = Adjacency::new();
        let mut missing = Vec::new();
        for artist in artists {
            match self.by_artist.get(artist) {
                Some(neighbors) => {
                    adjacency.insert(*artist, neighbors);
                }
                None => missing.push(*artist),
            }
        }
        let (loading, awaited) = self.claim(&missing);
        let loaded = self.load(pg, loading.artists.iter().map(|(artist, _)| *artist)).await;
        drop(loading);
        let Some(loaded) = loaded else {
            return Adjacency::new();
        };
        adjacency.extend(loaded);
        let mut stragglers = Vec::new();
        for (artist, done) in awaited {
            let _ = done.acquire().await;
            match self.by_artist.get(&artist) {
                Some(neighbors) => {
                    adjacency.insert(artist, neighbors);
                }
                None => stragglers.push(artist),
            }
        }
        let Some(loaded) = self.load(pg, stragglers).await else {
            return Adjacency::new();
        };
        adjacency.extend(loaded);
        adjacency.retain(|_, neighbors| !neighbors.is_empty());
        adjacency
    }

    fn claim(&self, missing: &[Uuid]) -> (Loading<'_>, Vec<(Uuid, Arc<Semaphore>)>) {
        let mut loading = self
            .loading
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut mine = Vec::new();
        let mut awaited = Vec::new();
        for artist in missing {
            match loading.get(artist) {
                Some(done) => awaited.push((*artist, done.clone())),
                None => {
                    let done = Arc::new(Semaphore::new(0));
                    loading.insert(*artist, done.clone());
                    mine.push((*artist, done));
                }
            }
        }
        (
            Loading {
                edges: self,
                artists: mine,
            },
            awaited,
        )
    }

    async fn load(
        &self,
        pg: &PgPool,
        artists: impl IntoIterator<Item = Uuid>,
    ) -> Option<Adjacency> {
        let artists: Vec<Uuid> = artists.into_iter().collect();
        if artists.is_empty() {
            return Some(Adjacency::new());
        }
        let edges = sqlx::query_file!(
            "queries/recommendations/smart_wave/graph/load_graph_edges.sql",
            &artists
        )
        .fetch_all(pg)
        .await
        .ok()?;
        let edges = edges
            .into_iter()
            .map(|edge| (edge.src, edge.dst, edge.weight, edge.kind));
        let mut adjacency = Adjacency::new();
        for (artist, neighbors) in neighbors_by_artist(&artists, edges) {
            let neighbors = Arc::new(neighbors);
            self.by_artist.insert(artist, neighbors.clone());
            adjacency.insert(artist, neighbors);
        }
        Some(adjacency)
    }
}

impl Default for GraphEdges {
    fn default() -> Self {
        Self::new()
    }
}

fn neighbors_by_artist(
    artists: &[Uuid],
    edges: impl IntoIterator<Item = (Uuid, Uuid, f32, i16)>,
) -> HashMap<Uuid, Neighbors> {
    let mut by_artist: HashMap<Uuid, Neighbors> = artists
        .iter()
        .map(|artist| (*artist, Neighbors::new()))
        .collect();
    for (src, dst, weight, kind) in edges {
        if let Some(neighbors) = by_artist.get_mut(&src) {
            neighbors.entry(kind).or_default().push((dst, weight));
        }
    }
    by_artist
}

fn edge_count(neighbors: &Neighbors) -> u32 {
    let edges: usize = neighbors.values().map(Vec::len).sum();
    u32::try_from(edges).unwrap_or(u32::MAX).saturating_add(1)
}

pub struct GraphResult {
    pub affinity: Affinity,
    pub disliked_artists: Vec<Uuid>,
}

pub async fn build_affinity(
    svc: &RecommendationsService,
    sc_user_id: &str,
    seed: GraphSeed,
) -> GraphResult {
    let variants = user_id_variants(sc_user_id);
    let disliked = load_disliked_artists(&svc.pg, &variants).await;

    if let GraphSeed::User = seed
        && let Some(cached) = read_cache(&svc.redis, sc_user_id).await
    {
        return GraphResult {
            affinity: cached,
            disliked_artists: disliked,
        };
    }

    let mut seeds = match seed {
        GraphSeed::User => {
            let mut s = load_user_seeds(&svc.pg, &variants).await;
            for v in s.values_mut() {
                *v = v.ln_1p();
            }
            s
        }
        GraphSeed::Track(t) => load_track_seeds(&svc.pg, t).await,
        GraphSeed::Artist(a) => {
            let mut m = HashMap::new();
            m.insert(a, 1.0f32);
            m
        }
    };
    for d in &disliked {
        seeds.remove(d);
    }
    if seeds.is_empty() {
        return GraphResult {
            affinity: HashMap::new(),
            disliked_artists: disliked,
        };
    }
    normalize_by_max(&mut seeds);

    let mut affinity = propagate(&svc.graph_edges, &svc.pg, &seeds, &disliked).await;
    anti_spread(&svc.graph_edges, &svc.pg, &mut affinity, &disliked).await;
    cap_top(&mut affinity, TOTAL_CAP);

    if let GraphSeed::User = seed {
        write_cache(&svc.redis, sc_user_id, &affinity).await;
    }
    GraphResult {
        affinity,
        disliked_artists: disliked,
    }
}

async fn propagate(
    edges: &GraphEdges,
    pg: &PgPool,
    seeds: &Affinity,
    disliked: &[Uuid],
) -> Affinity {
    let disliked_set: std::collections::HashSet<Uuid> = disliked.iter().copied().collect();
    let mut contribs: HashMap<Uuid, Vec<f32>> = HashMap::new();
    let mut seed_contribs: HashMap<Uuid, Vec<f32>> = HashMap::new();
    let mut activation = seeds.clone();

    for _ in 0..HOPS {
        if activation.is_empty() {
            break;
        }
        let frontier = top_keys(&activation, FRONTIER_CAP);
        let adjacency = edges.of(pg, &frontier).await;
        if adjacency.is_empty() {
            break;
        }

        let mut hop_contribs: HashMap<Uuid, Vec<f32>> = HashMap::new();
        for (src, kinds) in &adjacency {
            let Some(&act) = activation.get(src) else {
                continue;
            };
            for (dst, e) in merge_normalized(kinds) {
                if disliked_set.contains(&dst) {
                    continue;
                }
                if seeds.contains_key(&dst) {
                    seed_contribs.entry(dst).or_default().push(act * e * GAMMA);
                    continue;
                }
                hop_contribs.entry(dst).or_default().push(act * e * GAMMA);
            }
        }

        let mut next: Affinity = HashMap::new();
        for (dst, xs) in hop_contribs {
            let v = decay_fold(xs);
            if v >= EPS {
                contribs.entry(dst).or_default().push(v);
                next.insert(dst, v);
            }
        }
        if next.is_empty() {
            break;
        }
        activation = next;
    }

    let mut total = seeds.clone();
    for (k, xs) in contribs {
        total.insert(k, decay_fold(xs));
    }
    for (k, xs) in seed_contribs {
        if let Some(v) = total.get_mut(&k) {
            *v = v.max(decay_fold(xs)).min(1.0);
        }
    }
    total
}

fn decay_fold(mut xs: Vec<f32>) -> f32 {
    xs.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    let mut mult = 1.0f32;
    let mut sum = 0.0f32;
    for x in xs {
        sum += x * mult;
        mult *= PATH_DECAY;
    }
    sum.min(PROP_CAP)
}

async fn anti_spread(edges: &GraphEdges, pg: &PgPool, affinity: &mut Affinity, disliked: &[Uuid]) {
    if disliked.is_empty() {
        return;
    }
    for kinds in edges.of(pg, disliked).await.values() {
        for (dst, e) in merge_normalized(kinds) {
            if let Some(v) = affinity.get_mut(&dst) {
                *v = (*v - ANTISPREAD_MU * e).max(0.0);
            }
        }
    }
    for d in disliked {
        affinity.remove(d);
    }
    affinity.retain(|_, v| *v > 0.0);
}

fn merge_normalized(kinds: &HashMap<i16, Vec<(Uuid, f32)>>) -> HashMap<Uuid, f32> {
    let mut merged: HashMap<Uuid, f32> = HashMap::new();
    for dsts in kinds.values() {
        let max_w = dsts.iter().map(|(_, w)| *w).fold(0f32, f32::max).max(1e-6);
        for (dst, w) in dsts {
            let e = (w / max_w).clamp(0.0, 1.0);
            let cur = merged.entry(*dst).or_insert(0.0);
            if e > *cur {
                *cur = e;
            }
        }
    }
    merged
}

async fn load_user_seeds(pg: &PgPool, variants: &[String]) -> Affinity {
    let rows = sqlx::query_file!(
        "queries/recommendations/smart_wave/graph/load_user_seeds.sql",
        variants,
        LIKES_WINDOW_DAYS,
        PLAYS_WINDOW_DAYS,
        PLAY_BOOST,
        SEED_LIMIT,
        crate::modules::superlikes::SUPERLIKE_WEIGHT
    )
    .fetch_all(pg)
    .await
    .unwrap_or_default();
    rows.into_iter().map(|r| (r.artist_id, r.weight)).collect()
}

async fn load_track_seeds(pg: &PgPool, sc_track_id: u64) -> Affinity {
    let scid = sc_track_id.to_string();
    let rows = sqlx::query_file!(
        "queries/recommendations/smart_wave/graph/load_track_seeds.sql",
        &scid
    )
    .fetch_all(pg)
    .await
    .unwrap_or_default();
    if !rows.is_empty() {
        return rows.into_iter().map(|r| (r.artist_id, r.w)).collect();
    }
    let rows = sqlx::query_file!(
        "queries/recommendations/smart_wave/graph/load_track_seeds_via_album.sql",
        &scid
    )
    .fetch_all(pg)
    .await
    .unwrap_or_default();
    rows.into_iter().map(|r| (r.artist_id, r.w)).collect()
}

async fn load_disliked_artists(pg: &PgPool, variants: &[String]) -> Vec<Uuid> {
    sqlx::query_file_scalar!(
        "queries/recommendations/smart_wave/graph/load_disliked_artists.sql",
        variants,
        DISLIKE_ARTIST_MIN
    )
    .fetch_all(pg)
    .await
    .unwrap_or_default()
}

pub async fn collect_artist_tracks(
    pg: &PgPool,
    artist_ids: &[Uuid],
    exclude: &[String],
    per_artist: i64,
    total: i64,
) -> Vec<(u64, Uuid)> {
    if artist_ids.is_empty() {
        return Vec::new();
    }
    let rows = sqlx::query_file!(
        "queries/recommendations/smart_wave/graph/collect_artist_tracks.sql",
        artist_ids,
        exclude,
        per_artist,
        total
    )
    .fetch_all(pg)
    .await
    .unwrap_or_default();
    rows.into_iter()
        .filter_map(|r| r.sc_track_id.parse::<u64>().ok().map(|n| (n, r.artist_id)))
        .collect()
}

fn normalize_by_max(map: &mut Affinity) {
    let max = map.values().copied().fold(0f32, f32::max);
    if max <= 0.0 {
        return;
    }
    for v in map.values_mut() {
        *v = (*v / max).clamp(0.0, 1.0);
    }
}

fn top_keys(map: &Affinity, n: usize) -> Vec<Uuid> {
    let mut pairs: Vec<(Uuid, f32)> = map.iter().map(|(k, v)| (*k, *v)).collect();
    if pairs.len() > n {
        pairs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        pairs.truncate(n);
    }
    pairs.into_iter().map(|(k, _)| k).collect()
}

fn cap_top(map: &mut Affinity, n: usize) {
    if map.len() <= n {
        return;
    }
    let mut pairs: Vec<(Uuid, f32)> = map.drain().collect();
    pairs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    pairs.truncate(n);
    *map = pairs.into_iter().collect();
}

fn cache_key(sc_user_id: &str) -> String {
    format!("wave:graph2:{sc_user_id}")
}

pub async fn forget_user_affinity(redis: &RedisPool, sc_user_id: &str) {
    let Ok(mut conn) = redis.get().await else {
        return;
    };
    let _: Result<(), _> = conn.del::<_, ()>(cache_key(sc_user_id)).await;
}

async fn read_cache(redis: &RedisPool, sc_user_id: &str) -> Option<Affinity> {
    let mut conn = redis.get().await.ok()?;
    let raw: Option<String> = conn.get(cache_key(sc_user_id)).await.ok().flatten();
    let pairs: Vec<(Uuid, f32)> = serde_json::from_str(&raw?).ok()?;
    Some(pairs.into_iter().collect())
}

async fn write_cache(redis: &RedisPool, sc_user_id: &str, affinity: &Affinity) {
    let pairs: Vec<(Uuid, f32)> = affinity.iter().map(|(k, v)| (*k, *v)).collect();
    let Ok(payload) = serde_json::to_string(&pairs) else {
        return;
    };
    let Ok(mut conn) = redis.get().await else {
        return;
    };
    let _: Result<(), _> = conn
        .set_ex::<_, _, ()>(cache_key(sc_user_id), payload, CACHE_TTL_SECS)
        .await;
    debug!(user = %sc_user_id, artists = affinity.len(), "wave graph cached");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decay_fold_tz_case() {
        let v = decay_fold(vec![0.05, 0.45]);
        assert!((v - 0.475).abs() < 1e-6, "got {v}");
    }

    #[test]
    fn decay_fold_hub_stays_below_direct_neighbor() {
        let hub = decay_fold(vec![0.15; 15]);
        let direct = decay_fold(vec![0.45]);
        assert!(hub < 0.31, "hub={hub}");
        assert!(hub < direct);
    }

    #[test]
    fn decay_fold_capped_below_seed() {
        let v = decay_fold(vec![0.9, 0.9, 0.9, 0.9]);
        assert!((v - PROP_CAP).abs() < 1e-6, "got {v}");
    }

    #[test]
    fn decay_fold_empty_is_zero() {
        assert_eq!(decay_fold(Vec::new()), 0.0);
    }

    #[test]
    fn neighbors_are_grouped_under_their_artist_and_kind() {
        let artist = Uuid::from_u128(1);
        let lonely = Uuid::from_u128(2);
        let first = Uuid::from_u128(3);
        let second = Uuid::from_u128(4);

        let grouped = neighbors_by_artist(
            &[artist, lonely, artist],
            [
                (artist, first, 0.5, 0),
                (artist, second, 0.25, 1),
                (artist, first, 0.75, 1),
            ],
        );

        assert_eq!(grouped[&artist][&0], vec![(first, 0.5)]);
        assert_eq!(grouped[&artist][&1], vec![(second, 0.25), (first, 0.75)]);
        assert!(grouped[&lonely].is_empty());
        assert_eq!(edge_count(&grouped[&artist]), 4);
        assert_eq!(edge_count(&grouped[&lonely]), 1);
    }

    #[test]
    fn merge_normalized_per_kind_scales() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let mut kinds: HashMap<i16, Vec<(Uuid, f32)>> = HashMap::new();
        kinds.insert(0, vec![(a, 2.0), (b, 1.0)]);
        kinds.insert(1, vec![(a, 0.05), (b, 0.24)]);
        let m = merge_normalized(&kinds);
        assert!((m[&a] - 1.0).abs() < 1e-6);
        assert!((m[&b] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn merge_normalized_relative_within_kind() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let mut kinds: HashMap<i16, Vec<(Uuid, f32)>> = HashMap::new();
        kinds.insert(1, vec![(a, 0.24), (b, 0.12)]);
        let m = merge_normalized(&kinds);
        assert!((m[&a] - 1.0).abs() < 1e-6);
        assert!((m[&b] - 0.5).abs() < 1e-6);
    }

    async fn liked_artist(pool: &PgPool, track: &str, liked_days_ago: i32) -> anyhow::Result<Uuid> {
        let artist: Uuid = sqlx::query_scalar(
            "INSERT INTO artists (name, normalized_name, source) VALUES ($1, $1, 'test') RETURNING id",
        )
        .bind(format!("artist {track}"))
        .fetch_one(pool)
        .await?;
        sqlx::query(
            "WITH t AS (
                 INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms)
                 VALUES ($1, 'soundcloud:tracks:' || $1, 'Song', 'song', 200000) RETURNING id
             )
             INSERT INTO track_artists (artist_id, role, source, track_id)
             SELECT $2, 'primary', 'test', id FROM t",
        )
        .bind(track)
        .bind(artist)
        .execute(pool)
        .await?;
        sqlx::query(
            "INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state, created_at)
             VALUES ('17', $1, true, date_trunc('day', now()) - make_interval(days => $2))",
        )
        .bind(track)
        .bind(liked_days_ago)
        .execute(pool)
        .await?;
        Ok(artist)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_superliked_like_seeds_its_artist_three_times_harder_even_when_old(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let superliked = liked_artist(&pool, "1", 2).await?;
        let liked = liked_artist(&pool, "2", 2).await?;
        let old = liked_artist(&pool, "3", 400).await?;
        sqlx::query(
            "INSERT INTO user_superlikes (user_id, sc_track_id, like_created_at, created_at)
             SELECT user_id, sc_track_id, created_at, created_at FROM user_likes_tracks
             WHERE sc_track_id IN ('1', '3')",
        )
        .execute(&pool)
        .await?;

        let seeds = load_user_seeds(&pool, &user_id_variants("17")).await;

        let ratio = seeds[&superliked] / seeds[&liked];
        assert!((ratio - 3.0).abs() < 1e-3, "ratio {ratio}");
        assert!(seeds.contains_key(&old), "an old superlike stays a seed");
        Ok(())
    }
}
