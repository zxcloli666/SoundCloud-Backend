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
const CACHE_TTL_SECS: u64 = 600;
const EDGES_TTL: Duration = Duration::from_secs(3600);
const EDGES_KEPT: u64 = 4_000_000;
const EDGES_TOP_K_COPLAY: i64 = 256;
const EDGES_TOP_K_COLIKE: Option<i64> = None;
const EDGES_CACHE: &str = "graph_edges";

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
    top_k_coplay: i64,
    top_k_colike: Option<i64>,
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
        Self::with_top_k(EDGES_TOP_K_COPLAY, EDGES_TOP_K_COLIKE)
    }

    pub fn with_top_k(top_k_coplay: i64, top_k_colike: Option<i64>) -> Self {
        Self {
            by_artist: Cache::builder()
                .max_capacity(EDGES_KEPT)
                .weigher(|_, neighbors: &Arc<Neighbors>| edge_count(neighbors))
                .time_to_live(EDGES_TTL)
                .build(),
            loading: Mutex::new(HashMap::new()),
            top_k_coplay,
            top_k_colike,
        }
    }

    async fn of(&self, pg: &PgPool, artists: &[Uuid]) -> Adjacency {
        let mut adjacency = Adjacency::new();
        let mut missing = Vec::new();
        for artist in artists {
            match self.by_artist.get(artist) {
                Some(neighbors) => {
                    crate::metrics::record_cache(EDGES_CACHE, true);
                    adjacency.insert(*artist, neighbors);
                }
                None => {
                    crate::metrics::record_cache(EDGES_CACHE, false);
                    missing.push(*artist);
                }
            }
        }
        let (loading, awaited) = self.claim(&missing);
        let loaded = self
            .load(pg, loading.artists.iter().map(|(artist, _)| *artist))
            .await;
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
            &artists,
            self.top_k_coplay,
            self.top_k_colike
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
        crate::metrics::set_cache_size(
            EDGES_CACHE,
            self.by_artist.entry_count(),
            self.by_artist.weighted_size(),
        );
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

pub async fn edge_rows(
    pg: &PgPool,
    artists: &[Uuid],
    top_k_coplay: i64,
    top_k_colike: Option<i64>,
) -> Option<usize> {
    sqlx::query_file!(
        "queries/recommendations/smart_wave/graph/load_graph_edges.sql",
        artists,
        top_k_coplay,
        top_k_colike
    )
    .fetch_all(pg)
    .await
    .ok()
    .map(|rows| rows.len())
}

pub async fn legacy_edge_rows(pg: &PgPool, artists: &[Uuid]) -> Option<usize> {
    sqlx::query_file!(
        "queries/recommendations/smart_wave/graph/load_graph_edges_legacy.sql",
        artists
    )
    .fetch_all(pg)
    .await
    .ok()
    .map(|rows| rows.len())
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

    let affinity = affinity_from(&svc.graph_edges, &svc.pg, &variants, &disliked, &seed).await;

    if let GraphSeed::User = seed
        && !affinity.is_empty()
    {
        write_cache(&svc.redis, sc_user_id, &affinity).await;
    }
    GraphResult {
        affinity,
        disliked_artists: disliked,
    }
}

pub async fn affinity_from(
    edges: &GraphEdges,
    pg: &PgPool,
    variants: &[String],
    disliked: &[Uuid],
    seed: &GraphSeed,
) -> Affinity {
    let mut seeds = match seed {
        GraphSeed::User => {
            let mut s = load_user_seeds(pg, variants).await;
            for v in s.values_mut() {
                *v = v.ln_1p();
            }
            s
        }
        GraphSeed::Track(t) => load_track_seeds(pg, *t).await,
        GraphSeed::Artist(a) => {
            let mut m = HashMap::new();
            m.insert(*a, 1.0f32);
            m
        }
    };
    for d in disliked {
        seeds.remove(d);
    }
    if seeds.is_empty() {
        return HashMap::new();
    }
    normalize_by_max(&mut seeds);

    let mut affinity = propagate(edges, pg, &seeds, disliked).await;
    anti_spread(edges, pg, &mut affinity, disliked).await;
    cap_top(&mut affinity, TOTAL_CAP);
    affinity
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

pub async fn load_disliked_artists(pg: &PgPool, variants: &[String]) -> Vec<Uuid> {
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
    use sqlx::postgres::PgPoolOptions;

    const CALLERS: usize = 12;

    async fn wide(small: &PgPool) -> anyhow::Result<PgPool> {
        Ok(PgPoolOptions::new()
            .max_connections(40)
            .connect_with((*small.connect_options()).clone())
            .await?)
    }

    async fn seed(pool: &PgPool) -> anyhow::Result<[Uuid; 3]> {
        let artists = [1_u128, 2, 3].map(Uuid::from_u128);
        for (index, artist) in artists.iter().enumerate() {
            sqlx::query(
                "INSERT INTO artists (id, name, normalized_name, source)
                 VALUES ($1, $2, $2, 'test')",
            )
            .bind(artist)
            .bind(format!("graph artist {index}"))
            .execute(pool)
            .await?;
        }
        sqlx::query(
            "INSERT INTO artist_coplay (a_id, b_id, weight) VALUES ($1, $2, 0.5), ($1, $3, 0.25), ($2, $3, 0.75)",
        )
        .bind(artists[0])
        .bind(artists[1])
        .bind(artists[2])
        .execute(pool)
        .await?;
        sqlx::query("INSERT INTO artist_colike (a_id, b_id, co, w) VALUES ($1, $2, 5, 0.4)")
            .bind(artists[0])
            .bind(artists[1])
            .execute(pool)
            .await?;
        Ok(artists)
    }

    async fn waiting_on_edges(pool: &PgPool) -> anyhow::Result<i64> {
        Ok(sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity
             WHERE datname = current_database()
               AND wait_event_type = 'Lock'
               AND query LIKE '%FROM artist_coplay%'",
        )
        .fetch_one(pool)
        .await?)
    }

    async fn expected(pool: &PgPool, artists: &[Uuid]) -> anyhow::Result<Adjacency> {
        let edges = sqlx::query_file!(
            "queries/recommendations/smart_wave/graph/load_graph_edges.sql",
            artists,
            i64::MAX,
            None::<i64>
        )
        .fetch_all(pool)
        .await?;
        let edges = edges
            .into_iter()
            .map(|edge| (edge.src, edge.dst, edge.weight, edge.kind));
        let mut adjacency: Adjacency = neighbors_by_artist(artists, edges)
            .into_iter()
            .map(|(artist, neighbors)| (artist, Arc::new(neighbors)))
            .collect();
        adjacency.retain(|_, neighbors| !neighbors.is_empty());
        Ok(adjacency)
    }

    type Flat = Vec<(Uuid, i16, Vec<(Uuid, u32)>)>;

    fn sorted(adjacency: &Adjacency) -> Flat {
        let mut flat = Vec::new();
        for (artist, neighbors) in adjacency {
            for (kind, list) in neighbors.iter() {
                let mut list: Vec<_> = list
                    .iter()
                    .map(|(dst, weight)| (*dst, weight.to_bits()))
                    .collect();
                list.sort();
                flat.push((*artist, *kind, list));
            }
        }
        flat.sort();
        flat
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn concurrent_misses_for_one_artist_make_one_db_call(
        small: PgPool,
    ) -> anyhow::Result<()> {
        let pool = wide(&small).await?;
        let artists = seed(&pool).await?;
        let want = expected(&pool, &artists[..2]).await?;
        let edges = Arc::new(GraphEdges::new());

        let mut blocker = pool.begin().await?;
        sqlx::query("LOCK TABLE artist_coplay IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *blocker)
            .await?;
        let calls: Vec<_> = (0..CALLERS)
            .map(|_| {
                let pool = pool.clone();
                let edges = edges.clone();
                tokio::spawn(async move { edges.of(&pool, &artists[..2]).await })
            })
            .collect();
        tokio::time::sleep(Duration::from_millis(700)).await;
        let waiting = waiting_on_edges(&pool).await?;
        blocker.rollback().await?;

        assert_eq!(waiting, 1);
        for call in calls {
            assert_eq!(sorted(&call.await?), sorted(&want));
        }

        let mut blocker = pool.begin().await?;
        sqlx::query("LOCK TABLE artist_coplay IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *blocker)
            .await?;
        let cached = tokio::time::timeout(Duration::from_secs(2), edges.of(&pool, &artists[..2]))
            .await
            .expect("a cached artist must not touch the database");
        assert_eq!(sorted(&cached), sorted(&want));
        blocker.rollback().await?;
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn overlapping_callers_load_each_artist_once(small: PgPool) -> anyhow::Result<()> {
        let pool = wide(&small).await?;
        let artists = seed(&pool).await?;
        let first = vec![artists[0], artists[1]];
        let second = vec![artists[1], artists[2]];
        let want_first = expected(&pool, &first).await?;
        let want_second = expected(&pool, &second).await?;
        let edges = Arc::new(GraphEdges::new());

        let mut blocker = pool.begin().await?;
        sqlx::query("LOCK TABLE artist_coplay IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *blocker)
            .await?;
        let one = {
            let (pool, edges, first) = (pool.clone(), edges.clone(), first.clone());
            tokio::spawn(async move { edges.of(&pool, &first).await })
        };
        tokio::time::sleep(Duration::from_millis(200)).await;
        let two = {
            let (pool, edges, second) = (pool.clone(), edges.clone(), second.clone());
            tokio::spawn(async move { edges.of(&pool, &second).await })
        };
        tokio::time::sleep(Duration::from_millis(500)).await;
        let waiting = waiting_on_edges(&pool).await?;
        blocker.rollback().await?;

        assert_eq!(waiting, 2, "the shared artist is not loaded twice");
        assert_eq!(sorted(&one.await?), sorted(&want_first));
        assert_eq!(sorted(&two.await?), sorted(&want_second));
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_failed_owner_does_not_strand_the_waiters(small: PgPool) -> anyhow::Result<()> {
        let pool = wide(&small).await?;
        let artists = seed(&pool).await?;
        let want = expected(&pool, &artists[..2]).await?;
        let edges = Arc::new(GraphEdges::new());

        let mut blocker = pool.begin().await?;
        sqlx::query("LOCK TABLE artist_coplay IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *blocker)
            .await?;
        let owner = {
            let (pool, edges) = (pool.clone(), edges.clone());
            tokio::spawn(async move { edges.of(&pool, &artists[..2]).await })
        };
        tokio::time::sleep(Duration::from_millis(200)).await;
        let waiter = {
            let (pool, edges) = (pool.clone(), edges.clone());
            tokio::spawn(async move { edges.of(&pool, &artists[..2]).await })
        };
        tokio::time::sleep(Duration::from_millis(200)).await;
        owner.abort();
        let _ = owner.await;
        blocker.rollback().await?;

        let got = tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .expect("the waiter must not hang")?;
        assert_eq!(sorted(&got), sorted(&want));
        Ok(())
    }

    const OLD_EDGES: &str = "SELECT a_id, b_id, weight::real, 0::int2 FROM artist_coplay
        WHERE a_id = ANY ($1) AND weight > 0
        UNION ALL SELECT b_id, a_id, weight::real, 0::int2 FROM artist_coplay
        WHERE b_id = ANY ($1) AND weight > 0
        UNION ALL SELECT a_id, b_id, w, 1::int2 FROM artist_colike
        WHERE a_id = ANY ($1) AND w > 0
        UNION ALL SELECT b_id, a_id, w, 1::int2 FROM artist_colike
        WHERE b_id = ANY ($1) AND w > 0";

    const LOW_HUB: u128 = 1;
    const HIGH_HUB: u128 = 1000;
    const NEIGHBORS: u128 = 60;

    type Edge = (Uuid, Uuid, u32, i16);

    async fn hubs(pool: &PgPool) -> anyhow::Result<Vec<Uuid>> {
        let ids: Vec<Uuid> = std::iter::once(LOW_HUB)
            .chain(100..100 + NEIGHBORS)
            .chain(std::iter::once(HIGH_HUB))
            .map(Uuid::from_u128)
            .collect();
        sqlx::query(
            "INSERT INTO artists (id, name, normalized_name, source)
             SELECT id, id::text, id::text, 'test' FROM unnest($1::uuid[]) AS id",
        )
        .bind(&ids)
        .execute(pool)
        .await?;
        for (rank, neighbor) in (100..100 + NEIGHBORS).enumerate() {
            let weight = 1.0 + rank as f32 / 100.0;
            let (low, high) = (Uuid::from_u128(LOW_HUB), Uuid::from_u128(neighbor));
            let top = Uuid::from_u128(HIGH_HUB);
            for (a, b, w) in [(low, high, weight), (high, top, weight + 0.005)] {
                sqlx::query("INSERT INTO artist_coplay (a_id, b_id, weight) VALUES ($1, $2, $3)")
                    .bind(a)
                    .bind(b)
                    .bind(w)
                    .execute(pool)
                    .await?;
                sqlx::query("INSERT INTO artist_colike (a_id, b_id, co, w) VALUES ($1, $2, 5, $3)")
                    .bind(a)
                    .bind(b)
                    .bind(w / 10.0)
                    .execute(pool)
                    .await?;
            }
        }
        Ok(ids)
    }

    fn flat(rows: impl IntoIterator<Item = (Uuid, Uuid, f32, i16)>) -> Vec<Edge> {
        let mut rows: Vec<Edge> = rows
            .into_iter()
            .map(|(src, dst, weight, kind)| (src, dst, weight.to_bits(), kind))
            .collect();
        rows.sort();
        rows
    }

    async fn new_edges(
        pool: &PgPool,
        artists: &[Uuid],
        k: i64,
        colike: Option<i64>,
    ) -> anyhow::Result<Vec<Edge>> {
        let rows = sqlx::query_file!(
            "queries/recommendations/smart_wave/graph/load_graph_edges.sql",
            artists,
            k,
            colike
        )
        .fetch_all(pool)
        .await?;
        Ok(flat(
            rows.into_iter().map(|e| (e.src, e.dst, e.weight, e.kind)),
        ))
    }

    async fn old_edges(
        pool: &PgPool,
        artists: &[Uuid],
    ) -> anyhow::Result<Vec<(Uuid, Uuid, f32, i16)>> {
        Ok(sqlx::query_as(OLD_EDGES)
            .bind(artists)
            .fetch_all(pool)
            .await?)
    }

    fn cut_to_top(rows: &[(Uuid, Uuid, f32, i16)], k: usize) -> Vec<Edge> {
        let mut kept = Vec::new();
        let sources: std::collections::HashSet<Uuid> = rows.iter().map(|r| r.0).collect();
        for source in sources {
            for kind in [0_i16, 1] {
                let mut own: Vec<_> = rows
                    .iter()
                    .filter(|r| r.0 == source && r.3 == kind)
                    .collect();
                own.sort_by(|a, b| b.2.total_cmp(&a.2));
                let by_side = |low_side: bool| {
                    own.iter()
                        .filter(move |r| (source < r.1) == low_side)
                        .take(k)
                        .map(|r| **r)
                        .collect::<Vec<_>>()
                };
                kept.extend(by_side(true));
                kept.extend(by_side(false));
            }
        }
        flat(kept)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unbounded_top_k_returns_the_old_edges(pool: PgPool) -> anyhow::Result<()> {
        let ids = hubs(&pool).await?;
        let old = flat(old_edges(&pool, &ids).await?);
        assert!(old.len() > 4 * NEIGHBORS as usize);
        assert_eq!(new_edges(&pool, &ids, i64::MAX, None).await?, old);
        assert_eq!(new_edges(&pool, &ids, 10_000, Some(10_000)).await?, old);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_small_top_k_keeps_the_heaviest_edges_of_each_side(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let ids = hubs(&pool).await?;
        let old = old_edges(&pool, &ids).await?;
        for k in [1_usize, 7, 59, 60] {
            let new = new_edges(&pool, &ids, k as i64, Some(k as i64)).await?;
            assert_eq!(new, cut_to_top(&old, k), "k = {k}");
        }
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unlimited_colike_branch_returns_the_old_colike_rows_and_coplay_is_cut(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        let ids = hubs(&pool).await?;
        let old = old_edges(&pool, &ids).await?;
        let (colike, coplay): (Vec<_>, Vec<_>) = old.iter().cloned().partition(|r| r.3 == 1);
        for k in [1_usize, 7, 59] {
            let mut want = cut_to_top(&coplay, k);
            want.extend(flat(colike.clone()));
            want.sort();
            let new = new_edges(&pool, &ids, k as i64, None).await?;
            assert_eq!(new, want, "k = {k}");
            assert_eq!(
                new.iter().filter(|r| r.3 == 1).count(),
                colike.len(),
                "colike must not lose a row"
            );
        }
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn top_k_keeps_the_heaviest_edge_so_normalisation_is_unchanged(
        pool: PgPool,
    ) -> anyhow::Result<()> {
        hubs(&pool).await?;
        let hub = Uuid::from_u128(LOW_HUB);
        let full = GraphEdges::with_top_k(i64::MAX, None);
        let cut = GraphEdges::with_top_k(5, Some(5));
        let full = full.of(&pool, &[hub]).await;
        let cut = cut.of(&pool, &[hub]).await;
        let (full, cut) = (merge_normalized(&full[&hub]), merge_normalized(&cut[&hub]));
        assert_eq!(cut.len(), 5);
        assert!(cut.values().any(|e| *e == 1.0));
        for (dst, e) in &cut {
            assert_eq!(full[dst], *e);
        }
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_edge_query_walks_the_covering_indexes(pool: PgPool) -> anyhow::Result<()> {
        let ids = hubs(&pool).await?;
        sqlx::query("ANALYZE artist_coplay").execute(&pool).await?;
        sqlx::query("ANALYZE artist_colike").execute(&pool).await?;
        let mut tx = pool.begin().await?;
        sqlx::query("SET LOCAL enable_seqscan = off")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SET LOCAL enable_sort = off")
            .execute(&mut *tx)
            .await?;
        let plan: Vec<String> = sqlx::query_scalar(&format!(
            "EXPLAIN (COSTS OFF) {}",
            include_str!(
                "../../../../queries/recommendations/smart_wave/graph/load_graph_edges.sql"
            )
        ))
        .bind(&ids)
        .bind(50_i64)
        .bind(Some(50_i64))
        .fetch_all(&mut *tx)
        .await?;
        let plan = plan.join("\n");
        for index in [
            "artist_coplay_a_edges_idx",
            "artist_coplay_b_edges_idx",
            "artist_colike_a_edges_idx",
            "artist_colike_b_edges_idx",
        ] {
            assert!(plan.contains(index), "{index} missing in\n{plan}");
        }
        Ok(())
    }

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
