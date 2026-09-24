use std::collections::HashSet;

use tracing::info;
use uuid::Uuid;

use crate::error::AppResult;
use crate::modules::centroids::cosine;
use crate::qdrant::collections;

use super::clusters::{ClusterBuilder, ClusterNeighbor, ClusterResponse, pick_unique_ids};
use super::home_wave::merge_audio_pools;
use super::service::RecommendationsService;
use super::service::util::parse_id_or_null;
use super::smart_wave::{self, SmartWaveSeed};

const SAME_ARTIST_POOL: i64 = 60;
const FEATURED_LIMIT: i64 = 8;
const FANS_ALSO_LIMIT: usize = 120;
const SAME_VIBE_POOL: usize = 160;
const WAVE_LIMIT: usize = 24;

impl RecommendationsService {
    pub async fn similar_wave(
        &self,
        sc_track_id: &str,
        sc_user_id: &str,
        languages: Option<&[String]>,
        per_cluster: usize,
        hide_listened: bool,
    ) -> AppResult<ClusterResponse> {
        let per_cluster = per_cluster.clamp(4, 24);
        let anchor = match parse_id_or_null(sc_track_id) {
            Some(a) => a,
            None => return Ok(ClusterBuilder::new().finish()),
        };

        let primary_artist = self.load_primary_artist_id(sc_track_id).await;

        let seed = self.load_track_vectors(anchor).await;
        let mert_seed = seed.mert.clone();
        let clap_seed = seed.clap.clone();
        let lyrics_seed = seed.lyrics.clone();
        let collab_seed = seed.collab.clone();

        let exclude: Vec<String> = vec![sc_track_id.to_string()];

        let wave_fut = smart_wave::cluster_tracks(
            self,
            sc_user_id,
            languages,
            SmartWaveSeed::Track(anchor),
            WAVE_LIMIT,
            hide_listened,
        );

        let same_artist_fut = async {
            match primary_artist {
                Some(artist_id) => {
                    self.load_same_artist_tracks(
                        artist_id,
                        sc_track_id,
                        mert_seed.as_deref(),
                        per_cluster,
                    )
                    .await
                }
                None => (Vec::new(), HashSet::new()),
            }
        };

        let same_vibe_fut = async {
            let filter = self.build_filter(&exclude, languages);
            let mert_fut = async {
                if let Some(v) = &mert_seed {
                    self.search_by_vector(
                        collections::TRACKS_MERT,
                        v,
                        filter.as_ref(),
                        SAME_VIBE_POOL,
                    )
                    .await
                } else {
                    Vec::new()
                }
            };
            let clap_fut = async {
                if let Some(v) = &clap_seed {
                    self.search_by_vector(
                        collections::TRACKS_CLAP,
                        v,
                        filter.as_ref(),
                        SAME_VIBE_POOL / 2,
                    )
                    .await
                } else {
                    Vec::new()
                }
            };
            let lyrics_fut = async {
                if let Some(v) = &lyrics_seed {
                    self.search_by_vector(
                        collections::TRACKS_LYRICS,
                        v,
                        filter.as_ref(),
                        SAME_VIBE_POOL / 2,
                    )
                    .await
                } else {
                    Vec::new()
                }
            };
            let (mert_pool, clap_pool, lyrics_pool) = tokio::join!(mert_fut, clap_fut, lyrics_fut);
            merge_audio_pools(&mert_pool, &clap_pool, &lyrics_pool)
        };

        let featured_fut = self.load_featured_with(sc_track_id, FEATURED_LIMIT);

        let fans_also_fut = async {
            match &collab_seed {
                Some(v) => {
                    let filter = self.build_filter(&exclude, languages);
                    self.search_by_vector(
                        collections::TRACKS_COLLAB,
                        v,
                        filter.as_ref(),
                        FANS_ALSO_LIMIT,
                    )
                    .await
                }
                None => Vec::new(),
            }
        };

        let (wave_results, same_artist, same_vibe_pool, featured_raw, fans_also_pool) = tokio::join!(
            wave_fut,
            same_artist_fut,
            same_vibe_fut,
            featured_fut,
            fans_also_fut,
        );

        let (same_artist_ids, everything_by_the_artist) = same_artist;
        let mut builder = ClusterBuilder::new();
        builder.reserve(std::iter::once(sc_track_id.to_string()));
        builder.reserve(everything_by_the_artist.iter().cloned());
        let wave_ids = wave_results
            .iter()
            .map(|result| super::clusters::recommend_id_str(&result.id))
            .filter(|id| !id.is_empty())
            .collect();
        builder.push_observed("wave", wave_ids, &wave_results);
        builder.push_reserved("same_artist", same_artist_ids);

        let by_the_artist = self
            .also_by_the_artist(primary_artist, &same_vibe_pool, everything_by_the_artist)
            .await;
        let vibe_filtered = filter_vibe_pool(&same_vibe_pool, builder.taken(), &by_the_artist);
        let vibe_ids = pick_unique_ids(&vibe_filtered, builder.taken(), per_cluster);
        builder.push_observed("same_vibe", vibe_ids, &same_vibe_pool);

        let featured_filtered: Vec<ClusterNeighbor> = featured_raw
            .into_iter()
            .filter(|n| !builder.taken().contains(&n.track_id))
            .take(per_cluster)
            .collect();
        builder.push_with_neighbors("featured_with", featured_filtered);

        let fans_ids = pick_unique_ids(&fans_also_pool, builder.taken(), per_cluster);
        builder.push_observed("fans_also", fans_ids, &fans_also_pool);

        let missing = self
            .s3
            .find_missing(&builder.all_track_ids())
            .await
            .unwrap_or_default();
        builder.drop_missing(&missing);

        let result = builder.finish();
        self.record_impressions(
            sc_user_id,
            super::impressions::ImpressionSource::Similar,
            &result,
        )
        .await;
        info!(
            track = sc_track_id,
            clusters = result.clusters.len(),
            "similar_wave built"
        );
        Ok(result)
    }

    async fn also_by_the_artist(
        &self,
        artist: Option<Uuid>,
        pool: &[super::service::RecommendResult],
        mut known: HashSet<String>,
    ) -> HashSet<String> {
        let Some(artist) = artist else {
            return known;
        };
        let candidates: Vec<String> = pool
            .iter()
            .map(|result| super::clusters::recommend_id_str(&result.id))
            .filter(|id| !id.is_empty() && !known.contains(id))
            .collect();
        if candidates.is_empty() {
            return known;
        }
        let theirs: Vec<String> = sqlx::query_file_scalar!(
            "queries/recommendations/similar_wave/candidates_by_artist.sql",
            &candidates,
            artist
        )
        .fetch_all(&self.pg)
        .await
        .unwrap_or_default();
        known.extend(theirs);
        known
    }

    async fn load_primary_artist_id(&self, sc_track_id: &str) -> Option<Uuid> {
        sqlx::query_file_scalar!(
            "queries/recommendations/similar_wave/load_primary_artist_id.sql",
            sc_track_id
        )
        .fetch_optional(&self.pg)
        .await
        .ok()
        .flatten()
    }

    async fn load_same_artist_tracks(
        &self,
        artist_id: Uuid,
        anchor_track_id: &str,
        mert_seed: Option<&[f32]>,
        limit: usize,
    ) -> (Vec<String>, HashSet<String>) {
        let pool: Vec<String> = sqlx::query_file_scalar!(
            "queries/recommendations/similar_wave/load_same_artist_tracks.sql",
            artist_id,
            anchor_track_id,
            SAME_ARTIST_POOL
        )
        .fetch_all(&self.pg)
        .await
        .unwrap_or_default();
        if pool.is_empty() {
            return (Vec::new(), HashSet::new());
        }
        let everything_by_the_artist: HashSet<String> = pool.iter().cloned().collect();

        let Some(seed) = mert_seed else {
            return (
                pool.into_iter().take(limit).collect(),
                everything_by_the_artist,
            );
        };
        let numeric_ids: Vec<u64> = pool.iter().filter_map(|s| s.parse::<u64>().ok()).collect();
        let vec_map = self
            .retrieve_vectors(collections::TRACKS_MERT, &numeric_ids)
            .await;
        let mut scored: Vec<(String, f32)> = pool
            .into_iter()
            .filter_map(|id| {
                let v = vec_map.get(&id)?;
                Some((id, cosine(seed, v)))
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        (
            scored.into_iter().take(limit).map(|(id, _)| id).collect(),
            everything_by_the_artist,
        )
    }

    async fn load_featured_with(&self, anchor_track_id: &str, limit: i64) -> Vec<ClusterNeighbor> {
        let rows = sqlx::query_file!(
            "queries/recommendations/similar_wave/load_featured_with.sql",
            anchor_track_id,
            limit
        )
        .fetch_all(&self.pg)
        .await
        .unwrap_or_default();

        rows.into_iter()
            .map(|r| ClusterNeighbor {
                track_id: r.sc_track_id,
                artist_id: r.artist_id,
                artist_name: r.artist_name,
                avatar_url: r.avatar_url,
            })
            .collect()
    }
}

fn filter_vibe_pool(
    pool: &[super::service::RecommendResult],
    taken: &HashSet<String>,
    same_artist: &HashSet<String>,
) -> Vec<super::service::RecommendResult> {
    pool.iter()
        .filter(|r| {
            let id = super::clusters::recommend_id_str(&r.id);
            !id.is_empty() && !taken.contains(&id) && !same_artist.contains(&id)
        })
        .cloned()
        .collect()
}
