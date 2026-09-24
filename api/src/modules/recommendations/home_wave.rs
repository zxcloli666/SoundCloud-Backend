use std::collections::{HashMap, HashSet};

use chrono::Utc;
use tracing::info;
use uuid::Uuid;

use crate::error::AppResult;
use crate::qdrant::collections;

use super::bandits;
use super::clusters::{
    Cluster, ClusterBuilder, ClusterNeighbor, ClusterResponse, recommend_id_str,
};
use super::debias::ips_debias;
use super::impressions::ImpressionSource;
use super::quality;
use super::rerank_multi::RerankOptions;
use super::service::util::user_id_variants;
use super::service::{RecommendResult, RecommendationsService};
use super::sessions::mix_centroids;
use super::signal::{SeedKind, load_user_signals};
use super::smart_wave::{self, SmartWaveSeed};
use super::taste_vectors::TasteShelf;

const ALL_CLUSTERS: &[&str] = &[
    "wave",
    "top_artists",
    "adjacent",
    "fresh_drops",
    "same_vibe",
    "deep_cuts",
    "taste",
];

const WAVE_LIMIT: usize = 24;
const POOL_FOR_VIBE_DEEP: usize = 500;
const NEIGHBORS_TOP_LIMIT: i64 = 16;
const NEIGHBORS_ADJ_LIMIT: i64 = 20;
const FRESH_DROP_LIMIT: i64 = 24;
const RECENT_ARTISTS_LIMIT: i64 = 60;

pub struct HomeRequest {
    pub sc_user_id: String,
    pub languages: Option<Vec<String>>,
    pub per_cluster: usize,
    pub hide_listened: bool,
}

#[derive(Clone, Copy)]
enum ContentSpace {
    Clap,
    Lyrics,
}

impl ContentSpace {
    fn collection(self) -> &'static str {
        match self {
            Self::Clap => collections::TRACKS_CLAP,
            Self::Lyrics => collections::TRACKS_LYRICS,
        }
    }
}

#[derive(Clone, Copy)]
struct SearchInput<'a> {
    collection: &'static str,
    vector: &'a [f32],
}

struct MultimodalSearchInputs<'a> {
    mert: SearchInput<'a>,
    clap: Option<SearchInput<'a>>,
    lyrics: Option<SearchInput<'a>>,
}

fn multimodal_search_inputs<'a>(
    mert: &'a [f32],
    clap: Option<&'a [f32]>,
    lyrics: Option<&'a [f32]>,
) -> MultimodalSearchInputs<'a> {
    MultimodalSearchInputs {
        mert: SearchInput {
            collection: collections::TRACKS_MERT,
            vector: mert,
        },
        clap: clap.map(|vector| SearchInput {
            collection: collections::TRACKS_CLAP,
            vector,
        }),
        lyrics: lyrics.map(|vector| SearchInput {
            collection: collections::TRACKS_LYRICS,
            vector,
        }),
    }
}

impl RecommendationsService {
    pub async fn home_wave(&self, req: HomeRequest) -> AppResult<ClusterResponse> {
        let per_cluster = req.per_cluster.clamp(4, 28);
        let sc_user_id = req.sc_user_id.clone();
        let languages_vec = req.languages.clone();
        let languages = languages_vec.as_deref();

        let signals = load_user_signals(&self.pg, &sc_user_id).await?;

        if matches!(signals.best_seed_kind(), SeedKind::ColdStart) && !signals.has_any_signal() {
            return self
                .cold_start_response(languages, per_cluster, &sc_user_id)
                .await;
        }

        let hidden_listen = if req.hide_listened {
            super::smart_wave::signals::load_hidden_by_listen(
                &self.pg,
                &user_id_variants(&sc_user_id),
            )
            .await
        } else {
            Vec::new()
        };
        let exclude_set: HashSet<String> = hidden_listen
            .iter()
            .chain(signals.disliked_ids.iter())
            .cloned()
            .collect();
        let exclude_vec: Vec<String> = exclude_set.iter().cloned().collect();

        let seeds = signals.positive_seed();
        let taste_modes_fut = self.build_taste_modes(&seeds);
        let clap_centroid_fut = self.build_content_centroid(&seeds, ContentSpace::Clap);
        let lyrics_centroid_fut = self.build_content_centroid(&seeds, ContentSpace::Lyrics);
        let session_fut = self.detect_current_session(&sc_user_id);
        let hour_fut = self.hour_context(&sc_user_id, Utc::now());
        let anti_fut = self.build_anti_centroid_from_negatives(&signals.negatives);
        let bandits_fut = bandits::load_stats(&self.pg, &sc_user_id);
        let taste_pool_fut = self.taste_pool(&sc_user_id, &exclude_vec);
        let wave_fut = Box::pin(smart_wave::cluster_tracks(
            self,
            &sc_user_id,
            languages,
            SmartWaveSeed::User,
            WAVE_LIMIT,
            req.hide_listened,
        ));

        let (
            taste_modes,
            clap_centroid,
            lyrics_centroid,
            session_ctx,
            hour_ctx,
            anti_centroid,
            bandit_stats,
            taste_pool,
            wave_results,
        ) = tokio::join!(
            taste_modes_fut,
            clap_centroid_fut,
            lyrics_centroid_fut,
            session_fut,
            hour_fut,
            anti_fut,
            bandits_fut,
            taste_pool_fut,
            wave_fut,
        );
        let session_ctx = session_ctx.unwrap_or(None);
        let hour_ctx = hour_ctx.unwrap_or(None);
        let bandit_stats = bandit_stats.unwrap_or_default();

        let overall_centroid = taste_modes.first().map(|m| m.centroid.clone());
        let mixed_for_search = mix_centroids(
            overall_centroid.as_deref(),
            session_ctx.as_ref().map(|s| s.centroid.as_slice()),
            hour_ctx.as_ref().map(|h| h.centroid.as_slice()),
        );

        let recent_artists = self
            .recent_artists(&sc_user_id, RECENT_ARTISTS_LIMIT)
            .await
            .unwrap_or_default();

        let mut builder = ClusterBuilder::new();
        builder.reserve(exclude_vec.iter().cloned());
        let wave_ids = wave_results
            .iter()
            .map(|result| recommend_id_str(&result.id))
            .filter(|id| !id.is_empty())
            .collect();
        builder.push_observed("wave", wave_ids, &wave_results);

        let top_artists = self
            .load_top_artists_cluster(&sc_user_id, builder.taken(), NEIGHBORS_TOP_LIMIT)
            .await;
        builder.push_with_neighbors(
            "top_artists",
            dedupe_neighbors(top_artists, builder.taken(), per_cluster),
        );

        let adjacent = self
            .load_adjacent_artists_cluster(&sc_user_id, builder.taken(), NEIGHBORS_ADJ_LIMIT)
            .await;
        builder.push_with_neighbors(
            "adjacent",
            dedupe_neighbors(adjacent, builder.taken(), per_cluster),
        );

        let fresh = self
            .load_fresh_drops(&sc_user_id, builder.taken(), FRESH_DROP_LIMIT)
            .await;
        builder.push(
            "fresh_drops",
            fresh
                .into_iter()
                .filter(|id| !builder.taken().contains(id))
                .take(per_cluster)
                .collect(),
        );

        if seeds.is_empty() {
            let pool = self
                .cold_start_pool(languages, per_cluster * 4)
                .await
                .unwrap_or_default();
            builder.push(
                "discover",
                pool.into_iter()
                    .filter(|id| !builder.taken().contains(id))
                    .take(per_cluster)
                    .collect(),
            );
        }

        let taste_version = taste_pool.as_ref().map(|pool| pool.version.clone());
        if let Some(pool) = taste_pool {
            let taste_results = self
                .build_taste_shelf(
                    pool,
                    TasteShelf {
                        taken: builder.taken(),
                        per_cluster,
                        anti_centroid: anti_centroid.as_deref(),
                        recent_artists: &recent_artists,
                        user_centroid: overall_centroid.as_deref(),
                    },
                )
                .await;
            let taste_ids = taste_results
                .iter()
                .map(|result| recommend_id_str(&result.id))
                .filter(|id| !id.is_empty())
                .collect();
            builder.push_observed("taste", taste_ids, &taste_results);
        }

        let (vibe_results, deep_results) = match mixed_for_search.as_deref() {
            Some(centroid) => {
                self.build_vibe_and_deep(
                    centroid,
                    clap_centroid.as_deref(),
                    lyrics_centroid.as_deref(),
                    &exclude_vec,
                    languages,
                    builder.taken(),
                    per_cluster,
                    anti_centroid.as_deref(),
                    &recent_artists,
                    overall_centroid.as_deref(),
                )
                .await
            }
            None => (Vec::new(), Vec::new()),
        };
        let vibe_ids = vibe_results
            .iter()
            .map(|result| recommend_id_str(&result.id))
            .filter(|id| !id.is_empty())
            .collect();
        builder.push_observed("same_vibe", vibe_ids, &vibe_results);
        let deep_ids = deep_results
            .iter()
            .map(|result| recommend_id_str(&result.id))
            .filter(|id| !id.is_empty())
            .collect();
        builder.push_observed("deep_cuts", deep_ids, &deep_results);

        self.apply_quality_filter(&mut builder).await;

        let missing = self
            .s3
            .find_missing(&builder.all_track_ids())
            .await
            .unwrap_or_default();
        builder.drop_missing(&missing);

        let mut response = builder.finish();
        reorder_by_bandits(&mut response.clusters, &bandit_stats);

        let counts: Vec<(String, i64)> = response
            .clusters
            .iter()
            .map(|c| (c.id.to_string(), c.track_ids.len() as i64))
            .collect();
        if !counts.is_empty()
            && let Err(error) = bandits::record_shows(&self.pg, &sc_user_id, &counts).await
        {
            tracing::warn!(user = %sc_user_id, %error, "bandit show counts were not recorded");
        }

        self.record_impressions(&sc_user_id, ImpressionSource::Home, &response)
            .await;

        info!(
            user = %sc_user_id,
            clusters = response.clusters.len(),
            modes = taste_modes.len(),
            taste_version = taste_version.as_deref(),
            session = session_ctx.is_some(),
            hour = hour_ctx.is_some(),
            "home_wave built"
        );
        Ok(response)
    }

    pub async fn home_wave_coalesced(&self, req: HomeRequest) -> AppResult<String> {
        let lang = req
            .languages
            .as_ref()
            .map(|l| l.join(","))
            .unwrap_or_default();
        let key = format!(
            "home:{}:{}:{}:{}",
            req.sc_user_id, lang, req.per_cluster, req.hide_listened
        );
        self.cluster_flights
            .run(&key, || {
                let request = req;
                async move {
                    let response = Box::pin(self.home_wave(request)).await?;
                    Ok(serde_json::to_string(&response)
                        .unwrap_or_else(|_| String::from("{\"clusters\":[]}")))
                }
            })
            .await
    }

    pub async fn similar_wave_coalesced(
        &self,
        sc_track_id: &str,
        sc_user_id: &str,
        languages: Option<&[String]>,
        per_cluster: usize,
        hide_listened: bool,
    ) -> AppResult<String> {
        let lang = languages.map(|l| l.join(",")).unwrap_or_default();
        let key =
            format!("similar:{sc_user_id}:{sc_track_id}:{lang}:{per_cluster}:{hide_listened}");
        self.cluster_flights
            .run(&key, || async move {
                let response = Box::pin(self.similar_wave(
                    sc_track_id,
                    sc_user_id,
                    languages,
                    per_cluster,
                    hide_listened,
                ))
                .await?;
                Ok(serde_json::to_string(&response)
                    .unwrap_or_else(|_| String::from("{\"clusters\":[]}")))
            })
            .await
    }

    pub async fn artist_wave_coalesced(
        &self,
        artist_id: Uuid,
        sc_user_id: &str,
        per_cluster: usize,
        hide_listened: bool,
    ) -> AppResult<String> {
        let key = format!("artist:{sc_user_id}:{artist_id}:{per_cluster}:{hide_listened}");
        self.cluster_flights
            .run(&key, || async move {
                let response =
                    Box::pin(self.artist_wave(artist_id, sc_user_id, per_cluster, hide_listened))
                        .await?;
                Ok(serde_json::to_string(&response)
                    .unwrap_or_else(|_| String::from("{\"clusters\":[]}")))
            })
            .await
    }

    async fn cold_start_response(
        &self,
        languages: Option<&[String]>,
        per_cluster: usize,
        sc_user_id: &str,
    ) -> AppResult<ClusterResponse> {
        let pool = self.cold_start_pool(languages, per_cluster * 4).await?;
        let mut builder = ClusterBuilder::new();
        builder.push("discover", pool.into_iter().take(per_cluster).collect());
        self.apply_quality_filter(&mut builder).await;
        let missing = self
            .s3
            .find_missing(&builder.all_track_ids())
            .await
            .unwrap_or_default();
        builder.drop_missing(&missing);
        let response = builder.finish();
        self.record_impressions(sc_user_id, ImpressionSource::Home, &response)
            .await;
        Ok(response)
    }

    #[allow(clippy::too_many_arguments)]
    async fn build_vibe_and_deep(
        &self,
        centroid: &[f32],
        clap_centroid: Option<&[f32]>,
        lyrics_centroid: Option<&[f32]>,
        exclude: &[String],
        languages: Option<&[String]>,
        taken: &HashSet<String>,
        per_cluster: usize,
        anti_centroid: Option<&[f32]>,
        recent_artists: &HashSet<String>,
        user_centroid: Option<&[f32]>,
    ) -> (Vec<RecommendResult>, Vec<RecommendResult>) {
        let filter = self.build_filter(exclude, languages);
        let inputs = multimodal_search_inputs(centroid, clap_centroid, lyrics_centroid);
        let mert_fut = self.search_by_vector(
            inputs.mert.collection,
            inputs.mert.vector,
            filter.as_ref(),
            POOL_FOR_VIBE_DEEP,
        );
        let clap_fut = async {
            match inputs.clap {
                Some(input) => {
                    self.search_by_vector(
                        input.collection,
                        input.vector,
                        filter.as_ref(),
                        POOL_FOR_VIBE_DEEP / 2,
                    )
                    .await
                }
                None => Vec::new(),
            }
        };
        let lyrics_fut = async {
            match inputs.lyrics {
                Some(input) => {
                    self.search_by_vector(
                        input.collection,
                        input.vector,
                        filter.as_ref(),
                        POOL_FOR_VIBE_DEEP / 2,
                    )
                    .await
                }
                None => Vec::new(),
            }
        };
        let (mert_pool, clap_pool, lyrics_pool) = tokio::join!(mert_fut, clap_fut, lyrics_fut);
        let mut pool = merge_audio_pools(&mert_pool, &clap_pool, &lyrics_pool);
        if pool.is_empty() {
            return (Vec::new(), Vec::new());
        }
        self.attach_playback_counts(&mut pool).await;
        ips_debias(&mut pool);

        let vibe_pool: Vec<RecommendResult> = pool
            .iter()
            .filter(|r| !taken.contains(&recommend_id_str(&r.id)))
            .cloned()
            .collect();

        let vibe_ranked = self
            .rerank_multi(
                vibe_pool,
                RerankOptions {
                    limit: per_cluster,
                    diversity: 0.35,
                    novelty: 0.15,
                    serendipity: 0.05,
                    anti_centroid: anti_centroid.map(|a| a.to_vec()),
                    recent_artists: recent_artists.clone(),
                    user_centroid: user_centroid.map(|v| v.to_vec()),
                },
            )
            .await;
        let vibe_ids: Vec<String> = vibe_ranked
            .iter()
            .take(per_cluster)
            .map(|r| recommend_id_str(&r.id))
            .collect();
        let vibe_set: HashSet<String> = vibe_ids.iter().cloned().collect();

        let deep_pool: Vec<RecommendResult> = pool
            .into_iter()
            .filter(|r| {
                let id = recommend_id_str(&r.id);
                !taken.contains(&id) && !vibe_set.contains(&id)
            })
            .collect();

        let deep_ranked = self
            .rerank_multi(
                deep_pool,
                RerankOptions {
                    limit: per_cluster,
                    diversity: 0.55,
                    novelty: 0.25,
                    serendipity: 0.20,
                    anti_centroid: anti_centroid.map(|a| a.to_vec()),
                    recent_artists: recent_artists.clone(),
                    user_centroid: user_centroid.map(|v| v.to_vec()),
                },
            )
            .await;
        (
            vibe_ranked.into_iter().take(per_cluster).collect(),
            deep_ranked.into_iter().take(per_cluster).collect(),
        )
    }

    async fn build_content_centroid(
        &self,
        seeds: &[super::signal::WeightedTrack],
        space: ContentSpace,
    ) -> Option<Vec<f32>> {
        if seeds.is_empty() {
            return None;
        }
        let track_ids: Vec<u64> = seeds
            .iter()
            .filter_map(|s| s.sc_track_id.parse::<u64>().ok())
            .collect();
        if track_ids.is_empty() {
            return None;
        }
        let vec_map = self.retrieve_vectors(space.collection(), &track_ids).await;
        if vec_map.is_empty() {
            return None;
        }
        let first = vec_map.values().next()?;
        let dim = first.len();
        let mut acc = vec![0f32; dim];
        let mut total_w = 0f32;
        for s in seeds {
            if let Some(v) = vec_map.get(&s.sc_track_id) {
                let w = s.weight.max(0.05);
                for (i, x) in v.iter().enumerate().take(dim) {
                    acc[i] += x * w;
                }
                total_w += w;
            }
        }
        if total_w <= 0.0 {
            return None;
        }
        for x in acc.iter_mut() {
            *x /= total_w;
        }
        crate::modules::centroids::normalize(&mut acc);
        Some(acc)
    }

    async fn build_anti_centroid_from_negatives(
        &self,
        negatives: &[super::signal::WeightedTrack],
    ) -> Option<Vec<f32>> {
        if negatives.is_empty() {
            return None;
        }
        let numeric: Vec<u64> = negatives
            .iter()
            .filter_map(|n| n.sc_track_id.parse::<u64>().ok())
            .collect();
        if numeric.is_empty() {
            return None;
        }
        let vec_map = self
            .retrieve_vectors(collections::TRACKS_MERT, &numeric)
            .await;
        let weights: Vec<(String, f32)> = negatives
            .iter()
            .map(|n| (n.sc_track_id.clone(), n.weight.max(0.01)))
            .collect();
        super::taste_modes::build_anti_centroid(&vec_map, &weights)
    }

    async fn recent_artists(&self, sc_user_id: &str, limit: i64) -> AppResult<HashSet<String>> {
        let ids = user_id_variants(sc_user_id);
        let rows = sqlx::query_file_scalar!(
            "queries/recommendations/home_wave/recent_artists.sql",
            &ids,
            limit
        )
        .fetch_all(&self.pg)
        .await?;
        Ok(rows.into_iter().collect())
    }

    async fn load_top_artists_cluster(
        &self,
        sc_user_id: &str,
        exclude: &HashSet<String>,
        limit: i64,
    ) -> Vec<ClusterNeighbor> {
        let exclude_vec: Vec<String> = exclude.iter().cloned().collect();
        let ids = user_id_variants(sc_user_id);
        let rows = match sqlx::query_file!(
            "queries/recommendations/home_wave/top_artists_cluster.sql",
            &ids,
            limit,
            &exclude_vec
        )
        .fetch_all(&self.pg)
        .await
        {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        rows.into_iter()
            .map(|r| ClusterNeighbor {
                track_id: r.sc_track_id,
                artist_id: r.artist_id,
                artist_name: r.artist_name,
                avatar_url: r.avatar_url,
            })
            .collect()
    }

    async fn load_adjacent_artists_cluster(
        &self,
        sc_user_id: &str,
        exclude: &HashSet<String>,
        limit: i64,
    ) -> Vec<ClusterNeighbor> {
        let exclude_vec: Vec<String> = exclude.iter().cloned().collect();
        let ids = user_id_variants(sc_user_id);
        let rows = match sqlx::query_file!(
            "queries/recommendations/home_wave/adjacent_artists_cluster.sql",
            &ids,
            limit,
            &exclude_vec
        )
        .fetch_all(&self.pg)
        .await
        {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        rows.into_iter()
            .map(|r| ClusterNeighbor {
                track_id: r.sc_track_id,
                artist_id: r.artist_id,
                artist_name: r.artist_name,
                avatar_url: r.avatar_url,
            })
            .collect()
    }

    async fn load_fresh_drops(
        &self,
        sc_user_id: &str,
        exclude: &HashSet<String>,
        limit: i64,
    ) -> Vec<String> {
        let exclude_vec: Vec<String> = exclude.iter().cloned().collect();
        let ids = user_id_variants(sc_user_id);
        sqlx::query_file_scalar!(
            "queries/recommendations/home_wave/fresh_drops.sql",
            &ids,
            &exclude_vec,
            limit
        )
        .fetch_all(&self.pg)
        .await
        .unwrap_or_default()
    }

    async fn apply_quality_filter(&self, builder: &mut ClusterBuilder) {
        const QUALITY_THRESHOLD: f32 = 0.4;
        let all_ids = builder.all_track_ids();
        if all_ids.is_empty() {
            return;
        }
        let rows = sqlx::query_file!(
            "queries/recommendations/home_wave/quality_rows.sql",
            &all_ids
        )
        .fetch_all(&self.pg)
        .await
        .unwrap_or_default();

        let by_id: HashMap<String, (i32, String, i64, Option<f32>)> = rows
            .into_iter()
            .map(|r| {
                (
                    r.sc_track_id,
                    (
                        r.duration_ms,
                        r.title,
                        r.play_count.unwrap_or(0),
                        r.quality_score,
                    ),
                )
            })
            .collect();

        let to_drop: HashSet<String> = all_ids
            .into_iter()
            .filter(|id| {
                let Some((dur, title, plays, quality)) = by_id.get(id) else {
                    return true;
                };
                if let Some(q) = quality {
                    return *q < QUALITY_THRESHOLD;
                }
                !quality::passes(
                    quality::QualityCheck {
                        duration_ms: *dur,
                        title,
                        plays: *plays,
                    },
                    quality::MIN_PLAYS_DEFAULT,
                )
            })
            .collect();
        builder.drop_missing(&to_drop);
    }

    pub(crate) async fn attach_playback_counts(&self, pool: &mut [RecommendResult]) {
        if pool.is_empty() {
            return;
        }
        let ids: Vec<String> = pool
            .iter()
            .map(|r| recommend_id_str(&r.id))
            .filter(|s| !s.is_empty())
            .collect();
        if ids.is_empty() {
            return;
        }
        let rows = sqlx::query_file!(
            "queries/recommendations/home_wave/playback_counts.sql",
            &ids
        )
        .fetch_all(&self.pg)
        .await
        .unwrap_or_default();
        let by_id: HashMap<String, i64> = rows
            .into_iter()
            .map(|r| (r.sc_track_id, r.play_count.unwrap_or(0)))
            .collect();
        for r in pool.iter_mut() {
            let id = recommend_id_str(&r.id);
            if let Some(p) = by_id.get(&id) {
                r.playback_count = Some(*p);
            }
        }
    }
}

fn dedupe_neighbors(
    raw: Vec<ClusterNeighbor>,
    taken: &HashSet<String>,
    limit: usize,
) -> Vec<ClusterNeighbor> {
    let mut out = Vec::with_capacity(limit);
    let mut seen_artists: HashSet<Uuid> = HashSet::new();
    for n in raw {
        if out.len() >= limit {
            break;
        }
        if taken.contains(&n.track_id) {
            continue;
        }
        if !seen_artists.insert(n.artist_id) {
            continue;
        }
        out.push(n);
    }
    out
}

fn reorder_by_bandits(clusters: &mut [Cluster], stats: &HashMap<String, bandits::ClusterStat>) {
    if clusters.len() <= 1 {
        return;
    }
    let order: Vec<&str> = bandits::order_by_thompson(&ALL_CLUSTERS[1..], stats);
    let mut priority: HashMap<&str, usize> = HashMap::new();
    priority.insert("wave", 0);
    for (i, c) in order.into_iter().enumerate() {
        priority.insert(c, i + 1);
    }
    clusters.sort_by_key(|c| priority.get(c.id).copied().unwrap_or(usize::MAX));
}

pub(crate) fn merge_audio_pools(
    mert: &[RecommendResult],
    clap: &[RecommendResult],
    lyrics: &[RecommendResult],
) -> Vec<RecommendResult> {
    const W_MERT: f32 = 0.5;
    const W_CLAP: f32 = 0.3;
    const W_LYRICS: f32 = 0.2;

    fn add(
        acc: &mut HashMap<String, (f32, RecommendResult)>,
        pool: &[RecommendResult],
        weight: f32,
    ) {
        let n = pool.len();
        if n == 0 {
            return;
        }
        let mean: f32 = pool.iter().map(|r| r.score.unwrap_or(0.0)).sum::<f32>() / n as f32;
        let var: f32 = pool
            .iter()
            .map(|r| {
                let s = r.score.unwrap_or(0.0);
                (s - mean) * (s - mean)
            })
            .sum::<f32>()
            / n as f32;
        let std = var.sqrt().max(1e-6);
        for r in pool {
            let id = recommend_id_str(&r.id);
            if id.is_empty() {
                continue;
            }
            let z = (r.score.unwrap_or(0.0) - mean) / std;
            let entry = acc.entry(id).or_insert_with(|| (0.0, r.clone()));
            entry.0 += z * weight;
        }
    }

    let mut acc: HashMap<String, (f32, RecommendResult)> = HashMap::new();
    add(&mut acc, mert, W_MERT);
    add(&mut acc, clap, W_CLAP);
    add(&mut acc, lyrics, W_LYRICS);

    let mut out: Vec<RecommendResult> = acc
        .into_iter()
        .map(|(_, (score, mut r))| {
            r.score = Some(score);
            r
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .unwrap_or(0.0)
            .partial_cmp(&a.score.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}
