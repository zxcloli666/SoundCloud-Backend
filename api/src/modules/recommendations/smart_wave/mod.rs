pub mod blocked;
pub mod cursor;
pub mod graph;
pub mod rank;
pub mod session;
pub mod signals;
pub mod track_arm;

use std::collections::HashMap;
use std::collections::HashSet;

use deadpool_redis::Pool as RedisPool;
use deadpool_redis::redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tracing::{debug, info};
use uuid::Uuid;

use crate::error::AppResult;
use crate::modules::recommendations::service::util::user_id_variants;
use crate::modules::recommendations::service::{RecommendResult, RecommendationsService};
use crate::modules::superlikes::SUPERLIKE_WEIGHT;
use crate::modules::superlikes::service::FREE_LIMIT;
use crate::qdrant::collections;

use cursor::{SeedKind, WaveCursor};
use graph::GraphSeed;
use rank::TrackMeta;
use session::SessionFeedback;
use signals::UserSignals;

const ARTIST_CAP_IN_WINDOW: usize = 2;
const MERT_POOL: usize = 400;
const GRAPH_FLOOR: f32 = 0.12;
const CALM_NEG_RATE: f32 = 0.3;
const CONTENT_FLOOR: f32 = 0.55;
const SEED_MOOD_WEIGHT: f32 = 0.7;
const GRAPH_ARTISTS: usize = 160;
const GRAPH_PER_ARTIST: i64 = 6;
const GRAPH_TRACKS_TOTAL: i64 = 800;
const SEED_LIKES_USER: usize = 14;
const SEED_SUPERLIKES_USER: usize = FREE_LIMIT as usize;
const TASTE_LIKES: usize = 80;
const SEED_SERVED_FORWARD: usize = 8;
const SEED_LIKED_NOW: usize = 4;
const QDRANT_NEGATIVES: usize = 40;
const SESSION_PULL_USER: f32 = 0.4;
const SESSION_PULL_SEEDED: f32 = 0.25;

pub enum SmartWaveSeed<'a> {
    User,
    Track(u64),
    Artist(Uuid, &'a [u64]),
}

pub struct SmartWaveRequest<'a> {
    pub sc_user_id: &'a str,
    pub languages: Option<&'a [String]>,
    pub limit: usize,
    pub cursor_token: Option<&'a str>,
    pub seed: SmartWaveSeed<'a>,
    pub hide_listened: bool,
}

pub struct SmartWaveResponse {
    pub tracks: Vec<RecommendResult>,
    pub cursor: String,
}

pub async fn build(
    svc: &RecommendationsService,
    req: SmartWaveRequest<'_>,
) -> AppResult<SmartWaveResponse> {
    let (signals, hidden_listen) = tokio::join!(
        signals::load_recent_signals(&svc.pg, req.sc_user_id),
        async {
            if req.hide_listened {
                signals::load_hidden_by_listen(&svc.pg, &user_id_variants(req.sc_user_id)).await
            } else {
                Vec::new()
            }
        },
    );
    let signals = signals?;

    let (seed_kind, graph_seed) = match &req.seed {
        SmartWaveSeed::User => (SeedKind::User, GraphSeed::User),
        SmartWaveSeed::Track(t) => (SeedKind::Track, GraphSeed::Track(*t)),
        SmartWaveSeed::Artist(a, _) => (SeedKind::Artist, GraphSeed::Artist(*a)),
    };
    let seed_key = match &req.seed {
        SmartWaveSeed::User => req.sc_user_id.to_string(),
        SmartWaveSeed::Track(t) => format!("t{t}"),
        SmartWaveSeed::Artist(a, _) => format!("a{a}"),
    };
    let owner = if req.sc_user_id.is_empty() {
        "anon"
    } else {
        req.sc_user_id
    };
    let mut wave_cursor =
        cursor::load_or_new(&svc.redis, owner, req.cursor_token, seed_kind, &seed_key).await;
    let session = SessionFeedback::from_signals(&signals, &wave_cursor);
    let session_ids = session.track_ids();

    let mert_seeds_raw = pick_mert_seeds(&req.seed, &signals, &wave_cursor, &session);
    let boosted = superliked_ids(&signals);
    let seed_boost = mert_boost(&req.seed, &boosted);
    let exclude = build_exclude(&signals, &wave_cursor, &req.seed, &hidden_listen);
    let negative_raw = negative_ids_for_qdrant(&signals, &session);

    let (mert_seeds, negative_ids) = tokio::join!(
        filter_indexed(&svc.pg, &mert_seeds_raw),
        filter_indexed(&svc.pg, &negative_raw),
    );
    let filter = svc.build_filter(&exclude, req.languages);

    let graph_fut = graph::build_affinity(svc, req.sc_user_id, graph_seed);
    let mert_fut = track_arm::recommend_from_many(
        svc,
        &mert_seeds,
        &seed_boost,
        &negative_ids,
        filter.as_ref(),
        MERT_POOL,
    );
    let (graph_res, mert, session_meta) =
        tokio::join!(graph_fut, mert_fut, load_track_meta(&svc.pg, &session_ids));
    let mut affinity = graph_res.affinity;
    let mut disliked_set: HashSet<Uuid> = graph_res.disliked_artists.iter().copied().collect();
    let seed_artist = match &req.seed {
        SmartWaveSeed::Artist(a, _) => Some(*a),
        SmartWaveSeed::User | SmartWaveSeed::Track(_) => None,
    };
    session.steer_artists(&session_meta, seed_artist, &mut affinity, &mut disliked_set);

    let top_artists = top_affinity_artists(&affinity, GRAPH_ARTISTS);
    let graph_tracks = graph::collect_artist_tracks(
        &svc.pg,
        &top_artists,
        &exclude,
        GRAPH_PER_ARTIST,
        GRAPH_TRACKS_TOTAL,
    )
    .await;

    let pool_ids: Vec<u64> = mert.iter().map(|c| c.sc_track_id).collect();
    let meta = load_track_meta(&svc.pg, &pool_ids).await;

    let mut artist_of: HashMap<u64, Option<Uuid>> = HashMap::new();
    for (tid, aid) in &graph_tracks {
        artist_of.entry(*tid).or_insert(Some(*aid));
    }
    for c in &mert {
        if let Some(m) = meta.get(&c.sc_track_id)
            && m.storage_ok
        {
            artist_of.entry(c.sc_track_id).or_insert(m.primary_artist);
        }
    }

    let pooled: Vec<u64> = artist_of.keys().copied().collect();
    let blocked =
        blocked::blocked_uploads(&svc.pg, &user_id_variants(req.sc_user_id), &pooled).await;
    artist_of.retain(|tid, _| !blocked.contains(tid));
    let cand_ids: Vec<u64> = artist_of.keys().copied().collect();
    let liked_ids = taste_ids(&signals);
    let mood_seed_ids: Vec<u64> = match &req.seed {
        SmartWaveSeed::User => Vec::new(),
        SmartWaveSeed::Track(t) => vec![*t],
        SmartWaveSeed::Artist(_, tracks) => tracks.iter().take(20).copied().collect(),
    };
    let session_pull = match &req.seed {
        SmartWaveSeed::User => SESSION_PULL_USER,
        SmartWaveSeed::Track(_) | SmartWaveSeed::Artist(..) => SESSION_PULL_SEEDED,
    };
    let vector_ids: Vec<u64> = cand_ids.iter().chain(session_ids.iter()).copied().collect();
    let centroids_fut = mood_centroids(svc, req.sc_user_id, &mood_seed_ids, &liked_ids, &boosted);
    let cands_vecs_fut = async {
        tokio::join!(
            svc.retrieve_vectors(collections::TRACKS_MERT, &vector_ids),
            svc.retrieve_vectors(collections::TRACKS_CLAP, &vector_ids),
            svc.retrieve_vectors(collections::TRACKS_LYRICS, &vector_ids),
        )
    };
    let (taste, (cm, cc, cl)) = tokio::join!(centroids_fut, cands_vecs_fut);
    let taste = session.pull_taste(taste, [&cm, &cc, &cl], &boosted, session_pull);
    let skipped = session.skipped_vectors([&cm, &cc, &cl]);
    let cands: Vec<rank::Candidate> = artist_of
        .into_iter()
        .map(|(tid, artist)| {
            let key = tid.to_string();
            let content = geomean(&[
                sim(&taste.m, cm.get(&key)),
                sim(&taste.c, cc.get(&key)),
                sim(&taste.l, cl.get(&key)),
            ]);
            let near_skip = [
                sim(&skipped.m, cm.get(&key)),
                sim(&skipped.c, cc.get(&key)),
                sim(&skipped.l, cl.get(&key)),
            ];
            rank::Candidate {
                sc_track_id: tid,
                artist,
                content: session::away_from_skips(content, &near_skip),
            }
        })
        .collect();
    let cand_count = cands.len();

    let picked = rank::rank_and_pick(
        &cands,
        &affinity,
        &disliked_set,
        &wave_cursor,
        req.limit * 2,
        ARTIST_CAP_IN_WINDOW,
        graph_floor(wave_cursor.neg_rate()),
        CONTENT_FLOOR,
    );

    let ids_after: Vec<String> = picked.iter().map(|p| p.sc_track_id.to_string()).collect();
    let lang_allowed = svc
        .filter_tracks_by_language(&ids_after, req.languages)
        .await;

    let known_positives = session::positive_ids(&signals);
    let mut tracks: Vec<RecommendResult> = Vec::with_capacity(req.limit);
    for p in &picked {
        if tracks.len() >= req.limit {
            break;
        }
        let id_str = p.sc_track_id.to_string();
        if !lang_allowed.contains(&id_str) {
            continue;
        }
        tracks.push(RecommendResult {
            id: serde_json::json!(p.sc_track_id),
            score: Some(p.score),
            payload: None,
            artist: None,
            genre: None,
            playback_count: None,
            features: None,
        });
        wave_cursor.mark_served(p.sc_track_id, p.artist);
        if known_positives.contains(&p.sc_track_id) {
            wave_cursor.mark_known_positive(p.sc_track_id);
        }
    }

    let handle = cursor::save(&svc.redis, owner, &wave_cursor)
        .await
        .unwrap_or_else(|| wave_cursor.handle.clone());
    cursor::register_handle(&svc.redis, owner, &wave_cursor).await;

    info!(
        user = %req.sc_user_id,
        kind = ?seed_kind,
        served_total = wave_cursor.served,
        returned = tracks.len(),
        liked_now = session.positives.len(),
        skipped_now = session.negatives.len(),
        neg_rate = wave_cursor.neg_rate(),
        graph = affinity.len(),
        graph_tracks = graph_tracks.len(),
        mert_pool = mert.len(),
        taste = !taste.m.is_empty(),
        cands = cand_count,
        "wave built"
    );

    Ok(SmartWaveResponse {
        tracks,
        cursor: handle,
    })
}

pub async fn record_feedback(
    redis: &RedisPool,
    sc_user_id: &str,
    cursor_token: &str,
    negatives: usize,
    positives: usize,
) -> Option<String> {
    let owner = if sc_user_id.is_empty() {
        "anon"
    } else {
        sc_user_id
    };
    let mut wave_cursor = cursor::read(redis, owner, cursor_token).await?;
    wave_cursor.record_outcomes(negatives, positives);
    let handle = cursor::save(redis, owner, &wave_cursor).await?;
    cursor::register_handle(redis, owner, &wave_cursor).await;
    Some(handle)
}

pub async fn cluster_tracks(
    svc: &RecommendationsService,
    sc_user_id: &str,
    languages: Option<&[String]>,
    seed: SmartWaveSeed<'_>,
    limit: usize,
    hide_listened: bool,
) -> Vec<RecommendResult> {
    let req = SmartWaveRequest {
        sc_user_id,
        languages,
        limit,
        cursor_token: None,
        seed,
        hide_listened,
    };
    match Box::pin(build(svc, req)).await {
        Ok(resp) => resp.tracks,
        Err(e) => {
            debug!(error = %e, "wave: cluster_tracks failed");
            Vec::new()
        }
    }
}

fn build_exclude(
    signals: &UserSignals,
    cursor: &WaveCursor,
    seed: &SmartWaveSeed,
    hidden_listen: &[String],
) -> Vec<String> {
    let mut excl = signals.always_exclude();
    excl.extend(hidden_listen.iter().cloned());
    for t in cursor.seen_tracks.iter() {
        excl.push(t.to_string());
    }
    if let SmartWaveSeed::Track(t) = seed {
        excl.push(t.to_string());
    }
    excl.sort();
    excl.dedup();
    excl
}

fn pick_mert_seeds(
    seed: &SmartWaveSeed,
    signals: &UserSignals,
    cursor: &WaveCursor,
    session: &SessionFeedback,
) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    let fresh = signals.fresh_likes.iter();
    let liked_now = session.positives.iter().take(SEED_LIKED_NOW);
    match seed {
        SmartWaveSeed::Track(t) => {
            out.push(*t);
            for t in liked_now {
                push_seed(&mut out, *t);
            }
            push_parsed(&mut out, fresh.take(5), usize::MAX);
        }
        SmartWaveSeed::Artist(_, tracks) => {
            for t in tracks.iter().take(8) {
                push_seed(&mut out, *t);
            }
            for t in liked_now {
                push_seed(&mut out, *t);
            }
            push_parsed(&mut out, fresh.take(3), usize::MAX);
        }
        SmartWaveSeed::User => {
            for t in liked_now {
                push_seed(&mut out, *t);
            }
            let superliked = signals.superliked.iter().take(SEED_SUPERLIKES_USER);
            push_parsed(&mut out, superliked, usize::MAX);
            push_parsed(&mut out, fresh, SEED_LIKES_USER);
            push_parsed(&mut out, signals.recent_played.iter(), SEED_LIKES_USER);
        }
    }
    let served_forward = cursor
        .seen_tracks
        .iter()
        .rev()
        .filter(|t| !session.negatives.contains(t))
        .take(SEED_SERVED_FORWARD);
    for t in served_forward {
        push_seed(&mut out, *t);
    }
    out
}

fn push_seed(out: &mut Vec<u64>, id: u64) {
    if !out.contains(&id) {
        out.push(id);
    }
}

fn push_parsed<'a>(out: &mut Vec<u64>, ids: impl Iterator<Item = &'a String>, cap: usize) {
    for id in ids {
        if out.len() >= cap {
            break;
        }
        if let Ok(n) = id.parse::<u64>() {
            push_seed(out, n);
        }
    }
}

fn superliked_ids(signals: &UserSignals) -> HashSet<u64> {
    signals
        .superliked
        .iter()
        .filter_map(|id| id.parse::<u64>().ok())
        .collect()
}

fn mert_boost(seed: &SmartWaveSeed, boosted: &HashSet<u64>) -> HashSet<u64> {
    match seed {
        SmartWaveSeed::User => boosted.clone(),
        SmartWaveSeed::Track(_) | SmartWaveSeed::Artist(..) => HashSet::new(),
    }
}

fn taste_ids(signals: &UserSignals) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    let liked = signals
        .superliked
        .iter()
        .chain(signals.fresh_likes.iter().take(TASTE_LIKES));
    push_parsed(&mut out, liked, usize::MAX);
    out
}

fn seed_weight(id: &str, boosted: &HashSet<u64>) -> f32 {
    match id.parse::<u64>() {
        Ok(n) if boosted.contains(&n) => SUPERLIKE_WEIGHT,
        _ => 1.0,
    }
}

fn negative_ids_for_qdrant(signals: &UserSignals, session: &SessionFeedback) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    for id in session.negatives.iter().take(QDRANT_NEGATIVES) {
        push_seed(&mut out, *id);
    }
    let half = out.len() + (QDRANT_NEGATIVES - out.len()) / 2;
    push_parsed(&mut out, signals.recent_skips.iter(), half);
    push_parsed(&mut out, signals.disliked_ids.iter(), QDRANT_NEGATIVES);
    push_parsed(&mut out, signals.recent_skips.iter(), QDRANT_NEGATIVES);
    out
}

fn graph_floor(neg_rate: f32) -> f32 {
    GRAPH_FLOOR * (1.0 - (neg_rate - CALM_NEG_RATE).max(0.0))
}

const TASTE_TTL_SECS: u64 = 300;
const TASTE_CLUSTERS: usize = 1;

#[derive(Default)]
struct TasteCentroids {
    m: Vec<Vec<f32>>,
    c: Vec<Vec<f32>>,
    l: Vec<Vec<f32>>,
}

#[derive(Serialize, Deserialize)]
struct CachedTasteCentroids {
    m: Vec<Vec<f32>>,
    c: Vec<Vec<f32>>,
}

async fn taste_centroids(
    svc: &RecommendationsService,
    sc_user_id: &str,
    liked_ids: &[u64],
    boosted: &HashSet<u64>,
) -> TasteCentroids {
    let key = taste_key(sc_user_id, liked_ids, boosted);
    if !sc_user_id.is_empty()
        && let Some(cached) = read_taste_cache(&svc.redis, &key).await
    {
        let lyrics = svc
            .retrieve_vectors(collections::TRACKS_LYRICS, liked_ids)
            .await;
        return TasteCentroids {
            m: cached.m,
            c: cached.c,
            l: kmeans_centroids(&lyrics, TASTE_CLUSTERS, boosted),
        };
    }
    let (lm, lc, ll) = tokio::join!(
        svc.retrieve_vectors(collections::TRACKS_MERT, liked_ids),
        svc.retrieve_vectors(collections::TRACKS_CLAP, liked_ids),
        svc.retrieve_vectors(collections::TRACKS_LYRICS, liked_ids),
    );
    let cen = TasteCentroids {
        m: kmeans_centroids(&lm, TASTE_CLUSTERS, boosted),
        c: kmeans_centroids(&lc, TASTE_CLUSTERS, boosted),
        l: kmeans_centroids(&ll, TASTE_CLUSTERS, boosted),
    };
    if !sc_user_id.is_empty() && (!cen.m.is_empty() || !cen.c.is_empty() || !cen.l.is_empty()) {
        write_taste_cache(
            &svc.redis,
            &key,
            &CachedTasteCentroids {
                m: cen.m.clone(),
                c: cen.c.clone(),
            },
        )
        .await;
    }
    cen
}

async fn mood_centroids(
    svc: &RecommendationsService,
    sc_user_id: &str,
    seed_ids: &[u64],
    liked_ids: &[u64],
    boosted: &HashSet<u64>,
) -> TasteCentroids {
    let user = taste_centroids(svc, sc_user_id, liked_ids, boosted).await;
    if seed_ids.is_empty() {
        return user;
    }
    let (sm, sc, sl) = tokio::join!(
        svc.retrieve_vectors(collections::TRACKS_MERT, seed_ids),
        svc.retrieve_vectors(collections::TRACKS_CLAP, seed_ids),
        svc.retrieve_vectors(collections::TRACKS_LYRICS, seed_ids),
    );
    TasteCentroids {
        m: opt_to_centroids(blend_centroids(
            mean_centroid(&sm, boosted),
            centroids_mean(&user.m),
            SEED_MOOD_WEIGHT,
        )),
        c: opt_to_centroids(blend_centroids(
            mean_centroid(&sc, boosted),
            centroids_mean(&user.c),
            SEED_MOOD_WEIGHT,
        )),
        l: opt_to_centroids(blend_centroids(
            mean_centroid(&sl, boosted),
            centroids_mean(&user.l),
            SEED_MOOD_WEIGHT,
        )),
    }
}

fn opt_to_centroids(v: Option<Vec<f32>>) -> Vec<Vec<f32>> {
    v.into_iter().collect()
}

fn centroids_mean(cs: &[Vec<f32>]) -> Option<Vec<f32>> {
    let first = cs.first()?;
    let mut acc = vec![0.0f32; first.len()];
    for c in cs {
        for (a, b) in acc.iter_mut().zip(c.iter()) {
            *a += *b;
        }
    }
    let inv = 1.0 / cs.len() as f32;
    for a in acc.iter_mut() {
        *a *= inv;
    }
    Some(acc)
}

fn blend_centroids(seed: Option<Vec<f32>>, user: Option<Vec<f32>>, w: f32) -> Option<Vec<f32>> {
    match (seed, user) {
        (Some(s), Some(u)) => {
            let n = s.len().min(u.len());
            Some((0..n).map(|i| w * s[i] + (1.0 - w) * u[i]).collect())
        }
        (Some(s), None) => Some(s),
        (None, u) => u,
    }
}

fn taste_key(sc_user_id: &str, liked_ids: &[u64], boosted: &HashSet<u64>) -> String {
    let mut ids = liked_ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let mut digest = Sha256::new();
    for id in ids {
        digest.update(id.to_le_bytes());
    }
    let mut superliked: Vec<u64> = boosted.iter().copied().collect();
    superliked.sort_unstable();
    for id in superliked {
        digest.update(b"s");
        digest.update(id.to_le_bytes());
    }
    format!(
        "wave:taste5:{sc_user_id}:{}",
        hex::encode(digest.finalize())
    )
}

async fn read_taste_cache(redis: &RedisPool, key: &str) -> Option<CachedTasteCentroids> {
    let mut conn = redis.get().await.ok()?;
    let raw: Option<String> = conn.get(key).await.ok().flatten();
    serde_json::from_str(&raw?).ok()
}

async fn write_taste_cache(redis: &RedisPool, key: &str, centroids: &CachedTasteCentroids) {
    let Ok(payload) = serde_json::to_string(centroids) else {
        return;
    };
    let Ok(mut conn) = redis.get().await else {
        return;
    };
    let _: Result<(), _> = conn.set_ex::<_, _, ()>(key, payload, TASTE_TTL_SECS).await;
}

fn sim(centroids: &[Vec<f32>], vec: Option<&Vec<f32>>) -> Option<f32> {
    let v = vec?;
    centroids
        .iter()
        .map(|c| crate::modules::centroids::cosine(v, c))
        .fold(None, |acc: Option<f32>, s| {
            Some(acc.map_or(s, |a| a.max(s)))
        })
}

fn geomean(sims: &[Option<f32>]) -> f32 {
    let xs: Vec<f32> = sims
        .iter()
        .filter_map(|x| *x)
        .filter(|x| *x > 0.0)
        .collect();
    if xs.is_empty() {
        return 1.0;
    }
    let s: f32 = xs.iter().map(|x| x.ln()).sum();
    (s / xs.len() as f32).exp()
}

fn kmeans_centroids(
    vecs: &HashMap<String, Vec<f32>>,
    k: usize,
    boosted: &HashSet<u64>,
) -> Vec<Vec<f32>> {
    if vecs.is_empty() {
        return Vec::new();
    }
    let k = k.min(vecs.len() / 8).max(1);
    if k == 1 {
        return mean_centroid(vecs, boosted).into_iter().collect();
    }
    let mut ids: Vec<&String> = vecs.keys().collect();
    ids.sort();
    let points: Vec<(f32, &Vec<f32>)> = ids
        .into_iter()
        .filter_map(|id| Some((seed_weight(id, boosted), vecs.get(id)?)))
        .collect();
    let Some((_, first)) = points.first() else {
        return Vec::new();
    };
    let mut centers: Vec<Vec<f32>> = vec![(*first).clone()];
    while centers.len() < k {
        let far = points.iter().max_by(|a, b| {
            nearest_dist(a.1, &centers)
                .partial_cmp(&nearest_dist(b.1, &centers))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let Some((_, p)) = far else { break };
        centers.push((*p).clone());
    }
    for _ in 0..8 {
        let mut sums: Vec<(Vec<f32>, f32)> =
            centers.iter().map(|c| (vec![0.0; c.len()], 0.0)).collect();
        for (weight, p) in &points {
            let ci = nearest_center(p, &centers);
            let (s, total) = &mut sums[ci];
            for (a, b) in s.iter_mut().zip(p.iter()) {
                *a += weight * b;
            }
            *total += weight;
        }
        for (i, (s, total)) in sums.into_iter().enumerate() {
            if total > 0.0 {
                centers[i] = s.into_iter().map(|x| x / total).collect();
            }
        }
    }
    centers
}

fn nearest_dist(p: &[f32], centers: &[Vec<f32>]) -> f32 {
    centers
        .iter()
        .map(|c| 1.0 - crate::modules::centroids::cosine(p, c))
        .fold(f32::MAX, f32::min)
}

fn nearest_center(p: &[f32], centers: &[Vec<f32>]) -> usize {
    let mut best = 0usize;
    let mut best_d = f32::MAX;
    for (i, c) in centers.iter().enumerate() {
        let d = 1.0 - crate::modules::centroids::cosine(p, c);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

fn mean_centroid(vecs: &HashMap<String, Vec<f32>>, boosted: &HashSet<u64>) -> Option<Vec<f32>> {
    let dimensions = vecs.values().next()?.len();
    let mut acc = vec![0.0f32; dimensions];
    let mut total = 0.0f32;
    for (id, v) in vecs {
        let weight = seed_weight(id, boosted);
        for (a, b) in acc.iter_mut().zip(v.iter()) {
            *a += weight * b;
        }
        total += weight;
    }
    for a in acc.iter_mut() {
        *a /= total;
    }
    Some(acc)
}

fn top_affinity_artists(aff: &graph::Affinity, n: usize) -> Vec<Uuid> {
    let mut v: Vec<(Uuid, f32)> = aff.iter().map(|(k, w)| (*k, *w)).collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    v.truncate(n);
    v.into_iter().map(|(k, _)| k).collect()
}

async fn filter_indexed(pg: &PgPool, ids: &[u64]) -> Vec<u64> {
    if ids.is_empty() {
        return Vec::new();
    }
    let strs: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
    let rows: Vec<String> = sqlx::query_file_scalar!(
        "queries/recommendations/smart_wave/mod/filter_indexed.sql",
        &strs
    )
    .fetch_all(pg)
    .await
    .unwrap_or_default();
    let set: HashSet<String> = rows.into_iter().collect();
    ids.iter()
        .copied()
        .filter(|i| set.contains(&i.to_string()))
        .collect()
}

async fn load_track_meta(pg: &PgPool, ids: &[u64]) -> HashMap<u64, TrackMeta> {
    if ids.is_empty() {
        return HashMap::new();
    }
    let id_strs: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
    let rows = sqlx::query_file!(
        "queries/recommendations/smart_wave/mod/load_track_meta.sql",
        &id_strs
    )
    .fetch_all(pg)
    .await
    .unwrap_or_default();
    rows.into_iter()
        .filter_map(|r| {
            r.sc_track_id.parse::<u64>().ok().map(|n| {
                (
                    n,
                    TrackMeta {
                        primary_artist: r.primary_artist_id,
                        storage_ok: r.ok,
                    },
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vecs(points: &[(&str, Vec<f32>)]) -> HashMap<String, Vec<f32>> {
        points
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn kmeans_k1_is_mean() {
        let v = vecs(&[("1", vec![1.0, 0.0]), ("2", vec![0.0, 1.0])]);
        let cs = kmeans_centroids(&v, 1, &HashSet::new());
        assert_eq!(cs.len(), 1);
        assert!((cs[0][0] - 0.5).abs() < 1e-6 && (cs[0][1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn kmeans_few_points_stay_single_centroid() {
        let v = vecs(&[("1", vec![1.0, 0.0]), ("2", vec![0.0, 1.0])]);
        assert_eq!(kmeans_centroids(&v, 3, &HashSet::new()).len(), 1);
    }

    #[test]
    fn kmeans_deterministic_and_separates_modes() {
        let mut pts: Vec<(String, Vec<f32>)> = Vec::new();
        for i in 0..8 {
            pts.push((format!("x{i}"), vec![1.0, 0.05 * i as f32]));
            pts.push((format!("y{i}"), vec![0.05 * i as f32, 1.0]));
        }
        let v: HashMap<String, Vec<f32>> = pts.into_iter().collect();
        let a = kmeans_centroids(&v, 2, &HashSet::new());
        let b = kmeans_centroids(&v, 2, &HashSet::new());
        assert_eq!(a, b);
        assert_eq!(a.len(), 2);
        let cross = crate::modules::centroids::cosine(&a[0], &a[1]);
        assert!(cross < 0.8, "modes not separated: cos={cross}");
    }

    fn strings(range: std::ops::RangeInclusive<u64>) -> Vec<String> {
        range.map(|id| id.to_string()).collect()
    }

    #[test]
    fn superlikes_lead_the_user_seeds_within_the_same_budget() {
        let signals = UserSignals {
            superliked: vec!["900".into(), "901".into(), "12".into()],
            fresh_likes: strings(10..=40),
            recent_played: strings(50..=60),
            ..UserSignals::default()
        };
        let cursor = WaveCursor::new(SeedKind::User, "17".into());

        let seeds = pick_mert_seeds(
            &SmartWaveSeed::User,
            &signals,
            &cursor,
            &SessionFeedback::default(),
        );

        assert_eq!(seeds.len(), SEED_LIKES_USER);
        assert_eq!(&seeds[..4], &[900, 901, 12, 10]);
        assert_eq!(seeds.iter().filter(|id| **id == 12).count(), 1);
    }

    #[test]
    fn every_free_superlike_seeds_the_user_wave() {
        let superliked = strings(900..=911);
        let signals = UserSignals {
            superliked: superliked.clone(),
            fresh_likes: strings(10..=40),
            ..UserSignals::default()
        };
        let cursor = WaveCursor::new(SeedKind::User, "17".into());

        let seeds = pick_mert_seeds(
            &SmartWaveSeed::User,
            &signals,
            &cursor,
            &SessionFeedback::default(),
        );

        let expected: Vec<u64> = (900..910).chain(10..14).collect();
        assert_eq!(seeds, expected);
    }

    #[test]
    fn a_track_wave_keeps_its_seed_track_the_strongest() {
        let signals = UserSignals {
            superliked: vec!["900".into(), "11".into()],
            fresh_likes: strings(10..=20),
            ..UserSignals::default()
        };
        let cursor = WaveCursor::new(SeedKind::Track, "t7".into());
        let seed = SmartWaveSeed::Track(7);

        let seeds = pick_mert_seeds(&seed, &signals, &cursor, &SessionFeedback::default());
        let boost = mert_boost(&seed, &superliked_ids(&signals));
        let weights: Vec<f32> = seeds
            .iter()
            .enumerate()
            .map(|(position, id)| track_arm::seed_weight(position, seeds.len(), *id, &boost))
            .collect();

        assert_eq!(seeds, vec![7, 10, 11, 12, 13, 14]);
        assert!(weights[1..].iter().all(|weight| *weight < weights[0]));
    }

    fn served(seed_kind: SeedKind, ids: &[u64]) -> WaveCursor {
        let mut cursor = WaveCursor::new(seed_kind, "17".into());
        for id in ids {
            cursor.mark_served(*id, None);
        }
        cursor
    }

    #[test]
    fn what_you_liked_in_this_wave_leads_the_next_user_seeds() {
        let signals = UserSignals {
            superliked: vec!["900".into()],
            fresh_likes: strings(10..=40),
            recent_played: vec!["70".into()],
            ..UserSignals::default()
        };
        let cursor = served(SeedKind::User, &[60, 70, 80]);
        let session = SessionFeedback::from_signals(&signals, &cursor);

        let seeds = pick_mert_seeds(&SmartWaveSeed::User, &signals, &cursor, &session);

        assert_eq!(&seeds[..3], &[70, 900, 10]);
        assert_eq!(seeds.len(), SEED_LIKES_USER + 2);
    }

    #[test]
    fn a_track_wave_takes_what_you_liked_right_after_its_seed() {
        let signals = UserSignals {
            fresh_likes: strings(10..=20),
            recent_played: vec!["70".into()],
            ..UserSignals::default()
        };
        let cursor = served(SeedKind::Track, &[60, 70]);
        let session = SessionFeedback::from_signals(&signals, &cursor);
        let seed = SmartWaveSeed::Track(7);

        let seeds = pick_mert_seeds(&seed, &signals, &cursor, &session);
        let weights: Vec<f32> = seeds
            .iter()
            .enumerate()
            .map(|(position, id)| {
                track_arm::seed_weight(position, seeds.len(), *id, &HashSet::new())
            })
            .collect();

        assert_eq!(&seeds[..3], &[7, 70, 10]);
        assert!(weights[1..].iter().all(|weight| *weight < weights[0]));
    }

    #[test]
    fn an_artist_wave_keeps_its_artist_first_and_then_what_you_liked() {
        let signals = UserSignals {
            fresh_likes: strings(10..=20),
            recent_played: vec!["70".into()],
            ..UserSignals::default()
        };
        let cursor = served(SeedKind::Artist, &[70]);
        let session = SessionFeedback::from_signals(&signals, &cursor);
        let top: Vec<u64> = (100..110).collect();
        let seed = SmartWaveSeed::Artist(Uuid::from_u128(1), &top);

        let seeds = pick_mert_seeds(&seed, &signals, &cursor, &session);

        assert_eq!(&seeds[..8], &top[..8]);
        assert_eq!(&seeds[8..10], &[70, 10]);
    }

    #[test]
    fn a_skipped_track_never_seeds_the_next_page() {
        let signals = UserSignals {
            recent_skips: vec!["61".into()],
            disliked_ids: vec!["62".into()],
            ..UserSignals::default()
        };
        let cursor = served(SeedKind::Track, &[60, 61, 62, 63]);
        let session = SessionFeedback::from_signals(&signals, &cursor);

        let seeds = pick_mert_seeds(&SmartWaveSeed::Track(7), &signals, &cursor, &session);

        assert_eq!(seeds, vec![7, 63, 60]);
    }

    #[test]
    fn skips_of_this_wave_lead_the_qdrant_negatives_behind_any_number_of_dislikes() {
        let signals = UserSignals {
            disliked_ids: strings(1..=200),
            recent_skips: strings(500..=560),
            ..UserSignals::default()
        };
        let session = SessionFeedback {
            negatives: vec![999],
            ..SessionFeedback::default()
        };

        let negatives = negative_ids_for_qdrant(&signals, &session);

        assert_eq!(negatives.len(), QDRANT_NEGATIVES);
        assert_eq!(&negatives[..3], &[999, 500, 501]);
        assert!(
            negatives.contains(&1),
            "the newest dislikes still push away"
        );
    }

    #[test]
    fn a_bad_run_leans_the_wave_on_the_artist_graph() {
        assert!((graph_floor(0.0) - GRAPH_FLOOR).abs() < 1e-6);
        assert!((graph_floor(CALM_NEG_RATE) - GRAPH_FLOOR).abs() < 1e-6);
        assert!(graph_floor(0.8) < GRAPH_FLOOR * 0.6);
        assert!(graph_floor(1.0) > 0.0);
    }

    fn candidates(
        artist: u128,
        ids: std::ops::RangeInclusive<u64>,
        content: f32,
    ) -> Vec<rank::Candidate> {
        ids.map(|id| rank::Candidate {
            sc_track_id: id,
            artist: Some(Uuid::from_u128(artist)),
            content,
        })
        .collect()
    }

    fn next_page(
        cands: &[rank::Candidate],
        signals: &UserSignals,
        cursor: &mut WaveCursor,
        limit: usize,
    ) -> Vec<u64> {
        let session = SessionFeedback::from_signals(signals, cursor);
        let meta: HashMap<u64, TrackMeta> = cands
            .iter()
            .map(|c| {
                let meta = TrackMeta {
                    primary_artist: c.artist,
                    storage_ok: true,
                };
                (c.sc_track_id, meta)
            })
            .collect();
        let mut affinity: graph::Affinity = HashMap::from([
            (Uuid::from_u128(1), 1.0),
            (Uuid::from_u128(2), 0.5),
            (Uuid::from_u128(3), 0.4),
        ]);
        let mut disliked = HashSet::new();
        session.steer_artists(&meta, None, &mut affinity, &mut disliked);
        let picked = rank::rank_and_pick(
            cands,
            &affinity,
            &disliked,
            cursor,
            limit,
            ARTIST_CAP_IN_WINDOW,
            graph_floor(cursor.neg_rate()),
            CONTENT_FLOOR,
        );
        for p in &picked {
            cursor.mark_served(p.sc_track_id, None);
        }
        picked.iter().map(|p| p.sc_track_id).collect()
    }

    #[test]
    fn the_next_page_drops_a_twice_skipped_artist_and_pulls_toward_a_liked_one() {
        let cands: Vec<rank::Candidate> = [
            candidates(1, 10..=13, 0.8),
            candidates(2, 20..=22, 0.7),
            candidates(3, 30..=33, 0.9),
        ]
        .concat();
        let mut cursor = WaveCursor::new(SeedKind::User, "17".into());
        let first = next_page(&cands, &UserSignals::default(), &mut cursor, 5);
        assert_eq!(first, vec![10, 11, 30, 31, 20]);

        let unchanged = next_page(&cands, &UserSignals::default(), &mut cursor.clone(), 3);
        assert_eq!(unchanged, vec![12, 13, 32]);

        let feedback = UserSignals {
            recent_skips: vec!["10".into(), "11".into()],
            fresh_likes: vec!["20".into()],
            ..UserSignals::default()
        };
        let next = next_page(&cands, &feedback, &mut cursor, 3);
        assert_eq!(next, vec![21, 22, 32]);
    }

    #[tokio::test]
    #[ignore = "requires a local Redis"]
    async fn feedback_on_a_track_wave_keeps_that_wave_cursor() -> anyhow::Result<()> {
        let redis = deadpool_redis::Config::from_url(
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned()),
        )
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
        let mut wave = WaveCursor::new(SeedKind::Track, "t7".into());
        wave.mark_served(70, None);
        let Some(handle) = cursor::save(&redis, "17", &wave).await else {
            anyhow::bail!("redis is not reachable");
        };
        cursor::register_handle(&redis, "17", &wave).await;

        let updated = record_feedback(&redis, "17", &handle, 1, 0).await;
        let stored = cursor::read(&redis, "17", &handle).await;

        assert_eq!(updated.as_deref(), Some(handle.as_str()));
        let Some(stored) = stored else {
            anyhow::bail!("the track wave cursor is gone after feedback");
        };
        assert_eq!(stored.seed_kind, SeedKind::Track);
        assert!(stored.contains(70), "feedback dropped what the wave served");
        assert!((stored.neg_rate() - 1.0).abs() < 1e-6);
        assert_eq!(record_feedback(&redis, "17", "missing", 1, 0).await, None);
        Ok(())
    }

    #[test]
    fn a_superlike_weighs_three_times_in_the_user_wave_seeds() {
        let boosted: HashSet<u64> = [900].into();
        let boost = mert_boost(&SmartWaveSeed::User, &boosted);

        let superliked = track_arm::seed_weight(0, 10, 900, &boost);
        let liked = track_arm::seed_weight(0, 10, 10, &boost);

        assert!((superliked - SUPERLIKE_WEIGHT * liked).abs() < 1e-6);
    }

    #[test]
    fn a_superliked_track_pulls_the_taste_three_times_harder() {
        let v = vecs(&[("1", vec![1.0, 0.0]), ("2", vec![0.0, 1.0])]);
        let boosted: HashSet<u64> = [1].into();

        let mean = mean_centroid(&v, &boosted).unwrap_or_default();

        assert!((mean[0] - 0.75).abs() < 1e-6 && (mean[1] - 0.25).abs() < 1e-6);
        assert_ne!(
            taste_key("17", &[1, 2], &boosted),
            taste_key("17", &[1, 2], &HashSet::new()),
            "a new superlike must not be answered from the old taste"
        );
    }

    #[test]
    fn sim_takes_nearest_centroid() {
        let centroids = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let v = vec![0.0, 2.0];
        let s = sim(&centroids, Some(&v));
        assert!(s.is_some_and(|x| (x - 1.0).abs() < 1e-5));
        assert!(sim(&centroids, None).is_none());
    }

    #[test]
    fn geomean_conjunction() {
        let g = geomean(&[Some(0.9), Some(0.2), None]);
        assert!((g - (0.9f32 * 0.2).sqrt()).abs() < 1e-5);
        assert_eq!(geomean(&[None, None, None]), 1.0);
    }
}
