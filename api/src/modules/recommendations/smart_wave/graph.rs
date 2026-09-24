use std::collections::HashMap;

use deadpool_redis::Pool as RedisPool;
use deadpool_redis::redis::AsyncCommands;
use sqlx::PgPool;
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

pub enum GraphSeed {
    User,
    Track(u64),
    Artist(Uuid),
}

pub type Affinity = HashMap<Uuid, f32>;

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

    let mut affinity = propagate(&svc.pg, &seeds, &disliked).await;
    anti_spread(&svc.pg, &mut affinity, &disliked).await;
    cap_top(&mut affinity, TOTAL_CAP);

    if let GraphSeed::User = seed {
        write_cache(&svc.redis, sc_user_id, &affinity).await;
    }
    GraphResult {
        affinity,
        disliked_artists: disliked,
    }
}

async fn propagate(pg: &PgPool, seeds: &Affinity, disliked: &[Uuid]) -> Affinity {
    let disliked_set: std::collections::HashSet<Uuid> = disliked.iter().copied().collect();
    let mut contribs: HashMap<Uuid, Vec<f32>> = HashMap::new();
    let mut seed_contribs: HashMap<Uuid, Vec<f32>> = HashMap::new();
    let mut activation = seeds.clone();

    for _ in 0..HOPS {
        if activation.is_empty() {
            break;
        }
        let frontier = top_keys(&activation, FRONTIER_CAP);
        let edges = load_graph_edges(pg, &frontier).await;
        if edges.is_empty() {
            break;
        }

        let frontier_set: std::collections::HashSet<Uuid> = frontier.iter().copied().collect();
        let mut adjacency: HashMap<Uuid, HashMap<i16, Vec<(Uuid, f32)>>> = HashMap::new();
        for (a, b, w, kind) in edges {
            if frontier_set.contains(&a) {
                adjacency
                    .entry(a)
                    .or_default()
                    .entry(kind)
                    .or_default()
                    .push((b, w));
            }
            if frontier_set.contains(&b) {
                adjacency
                    .entry(b)
                    .or_default()
                    .entry(kind)
                    .or_default()
                    .push((a, w));
            }
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

async fn anti_spread(pg: &PgPool, affinity: &mut Affinity, disliked: &[Uuid]) {
    if disliked.is_empty() {
        return;
    }
    let edges = load_graph_edges(pg, disliked).await;
    let disliked_set: std::collections::HashSet<Uuid> = disliked.iter().copied().collect();
    let mut by_src: HashMap<Uuid, HashMap<i16, Vec<(Uuid, f32)>>> = HashMap::new();
    for (a, b, w, kind) in edges {
        if disliked_set.contains(&a) {
            by_src
                .entry(a)
                .or_default()
                .entry(kind)
                .or_default()
                .push((b, w));
        }
        if disliked_set.contains(&b) {
            by_src
                .entry(b)
                .or_default()
                .entry(kind)
                .or_default()
                .push((a, w));
        }
    }
    for (_, kinds) in by_src {
        for (dst, e) in merge_normalized(&kinds) {
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
        SEED_LIMIT
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

async fn load_graph_edges(pg: &PgPool, nodes: &[Uuid]) -> Vec<(Uuid, Uuid, f32, i16)> {
    if nodes.is_empty() {
        return Vec::new();
    }
    sqlx::query_file!(
        "queries/recommendations/smart_wave/graph/load_graph_edges.sql",
        nodes
    )
    .fetch_all(pg)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|r| (r.a_id, r.b_id, r.weight, r.kind))
            .collect()
    })
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
}
