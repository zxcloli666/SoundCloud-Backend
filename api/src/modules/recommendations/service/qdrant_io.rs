use deadpool_redis::redis::AsyncCommands;
use qdrant_client::qdrant::{
    Filter, GetPointsBuilder, PointId, SearchPointsBuilder, Value as QdrantValue,
    point_id::PointIdOptions, value::Kind as QdrantValueKind,
    vector_output::Vector as VectorVariant, vectors_output::VectorsOptions,
};
use std::collections::{HashMap, HashSet};
use tracing::debug;

const VEC_CACHE_TTL: u64 = 6 * 60 * 60;
pub(crate) const LYRICS_VECTOR_REQUEST_FIELD: &str = "embedding_request_id";

fn vec_cache_key(collection: &str, id: &str) -> String {
    format!("qv:{collection}:{id}")
}

pub(crate) fn lyrics_vec_cache_key(id: &str, request_id: &str) -> String {
    format!("qv:lyrics:v4:{id}:{request_id}")
}

fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(v.len() * 4);
    for x in v {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b
}

fn bytes_to_vec(b: &[u8]) -> Option<Vec<f32>> {
    if b.is_empty() || !b.len().is_multiple_of(4) {
        return None;
    }
    Some(
        b.chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

use super::RecommendationsService;
use super::types::RecommendResult;
use super::util::{numeric_id, payload_to_map, point_id_to_value, value_to_u64};

fn embedded_request(payload: &HashMap<String, QdrantValue>) -> Option<&String> {
    match payload.get(LYRICS_VECTOR_REQUEST_FIELD)?.kind.as_ref()? {
        QdrantValueKind::StringValue(request_id) => Some(request_id),
        _ => None,
    }
}

#[derive(Default)]
pub(crate) struct LyricsVectorEligibility {
    requests: HashMap<String, String>,
}

impl LyricsVectorEligibility {
    pub(crate) fn contains(&self, track_id: &str) -> bool {
        self.requests.contains_key(track_id)
    }

    pub(crate) fn matches(&self, track_id: &str, payload: &HashMap<String, QdrantValue>) -> bool {
        match (self.requests.get(track_id), embedded_request(payload)) {
            (Some(expected), Some(embedded_for)) => expected == embedded_for,
            _ => false,
        }
    }

    fn cache_key(&self, track_id: &str) -> Option<String> {
        self.requests
            .get(track_id)
            .map(|request_id| lyrics_vec_cache_key(track_id, request_id))
    }

    fn allows_all(&self, track_ids: &[String]) -> bool {
        track_ids.iter().all(|track_id| self.contains(track_id))
    }
}

impl RecommendationsService {
    pub(crate) async fn search_by_vector(
        &self,
        collection: &str,
        vector: &[f32],
        filter: Option<&Filter>,
        limit: usize,
    ) -> Vec<RecommendResult> {
        let mut builder =
            SearchPointsBuilder::new(collection, vector.to_vec(), limit as u64).with_payload(true);
        if let Some(f) = filter {
            builder = builder.filter(f.clone());
        }
        let raw = match Box::pin(self.qdrant.raw().search_points(builder)).await {
            Ok(r) => r.result,
            Err(e) => {
                debug!(collection, error = %e, "searchByVector failed");
                return Vec::new();
            }
        };
        let ids: Vec<String> = raw
            .iter()
            .filter_map(|p| value_to_u64(&point_id_to_value(p.id.clone())).map(|n| n.to_string()))
            .collect();
        let (public, lyrics_eligibility) = tokio::join!(
            self.public_track_ids(&ids),
            self.lyrics_vector_eligibility_for(collection, &ids)
        );
        raw.into_iter()
            .filter_map(|p| {
                let id = point_id_to_value(p.id);
                let id_str = value_to_u64(&id)?.to_string();
                if !public.contains(&id_str) {
                    return None;
                }
                if let Some(eligibility) = lyrics_eligibility.as_ref()
                    && !eligibility.matches(&id_str, &p.payload)
                {
                    return None;
                }
                Some(RecommendResult {
                    id,
                    score: Some(p.score),
                    payload: Some(payload_to_map(p.payload)),
                    artist: None,
                    genre: None,
                    playback_count: None,
                    features: None,
                })
            })
            .collect()
    }

    pub(crate) async fn public_track_ids(&self, ids: &[String]) -> HashSet<String> {
        load_public_track_ids(&self.pg, ids)
            .await
            .unwrap_or_default()
    }

    pub(crate) async fn cached_tracks_still_eligible(
        &self,
        track_ids: &[String],
        require_lyrics_vectors: bool,
    ) -> bool {
        if track_ids.is_empty() {
            return true;
        }
        if require_lyrics_vectors {
            let (public, lyrics) = tokio::join!(
                self.public_track_ids(track_ids),
                self.lyrics_vector_eligibility(track_ids)
            );
            return track_ids.iter().all(|track_id| public.contains(track_id))
                && lyrics.allows_all(track_ids);
        }
        let public = self.public_track_ids(track_ids).await;
        track_ids.iter().all(|track_id| public.contains(track_id))
    }

    pub(crate) async fn retrieve_vector(&self, collection: &str, id: u64) -> Option<Vec<f32>> {
        self.retrieve_vectors(collection, &[id])
            .await
            .remove(&id.to_string())
    }

    pub(crate) async fn retrieve_vectors(
        &self,
        collection: &str,
        ids: &[u64],
    ) -> HashMap<String, Vec<f32>> {
        retrieve_vectors_with(&self.qdrant, &self.redis, &self.pg, collection, ids).await
    }

    pub(crate) async fn lyrics_vector_eligibility(
        &self,
        ids: &[String],
    ) -> LyricsVectorEligibility {
        load_eligibility(&self.pg, ids).await
    }

    async fn lyrics_vector_eligibility_for(
        &self,
        collection: &str,
        ids: &[String],
    ) -> Option<LyricsVectorEligibility> {
        eligibility_for(&self.pg, collection, ids).await
    }
}

async fn load_eligibility(pool: &sqlx::PgPool, ids: &[String]) -> LyricsVectorEligibility {
    match load_lyrics_vector_eligibility(pool, ids).await {
        Ok(eligibility) => eligibility,
        Err(error) => {
            debug!(%error, "lyrics vector eligibility could not be loaded");
            LyricsVectorEligibility::default()
        }
    }
}

async fn eligibility_for(
    pool: &sqlx::PgPool,
    collection: &str,
    ids: &[String],
) -> Option<LyricsVectorEligibility> {
    if collection == crate::qdrant::collections::TRACKS_LYRICS {
        Some(load_eligibility(pool, ids).await)
    } else {
        None
    }
}

pub(crate) async fn retrieve_vectors_with(
    qdrant: &crate::qdrant::QdrantService,
    redis: &deadpool_redis::Pool,
    pg: &sqlx::PgPool,
    collection: &str,
    ids: &[u64],
) -> HashMap<String, Vec<f32>> {
    {
        let mut out: HashMap<String, Vec<f32>> = HashMap::new();
        if ids.is_empty() {
            return out;
        }
        let mut uniq: Vec<u64> = ids.to_vec();
        uniq.sort_unstable();
        uniq.dedup();

        let string_ids = uniq.iter().map(u64::to_string).collect::<Vec<_>>();
        let lyrics_eligibility = eligibility_for(pg, collection, &string_ids).await;
        if let Some(eligibility) = lyrics_eligibility.as_ref() {
            uniq.retain(|id| eligibility.contains(&id.to_string()));
            if uniq.is_empty() {
                return out;
            }
        }

        let mut misses: Vec<u64> = Vec::with_capacity(uniq.len());
        match redis.get().await {
            Ok(mut conn) => {
                let keys: Vec<String> = uniq
                    .iter()
                    .filter_map(|id| {
                        let id = id.to_string();
                        match lyrics_eligibility.as_ref() {
                            Some(eligibility) => eligibility.cache_key(&id),
                            None => Some(vec_cache_key(collection, &id)),
                        }
                    })
                    .collect();
                let cached: Vec<Option<Vec<u8>>> = conn.mget(&keys).await.unwrap_or_default();
                if cached.len() == uniq.len() {
                    for (id, c) in uniq.iter().zip(cached) {
                        match c.as_deref().and_then(bytes_to_vec) {
                            Some(v) => {
                                out.insert(id.to_string(), v);
                            }
                            None => misses.push(*id),
                        }
                    }
                } else {
                    misses = uniq.clone();
                }
            }
            Err(_) => misses = uniq.clone(),
        }
        if misses.is_empty() {
            return out;
        }

        let pids: Vec<PointId> = misses.iter().copied().map(numeric_id).collect();
        let mut request = GetPointsBuilder::new(collection, pids).with_vectors(true);
        if lyrics_eligibility.is_some() {
            request = request.with_payload(true);
        }
        let fetched = match qdrant.raw().get_points(request).await {
            Ok(r) => r.result,
            Err(e) => {
                debug!(collection, error = %e, "retrieveVectors failed");
                return out;
            }
        };
        let mut to_cache: Vec<(String, Vec<u8>)> = Vec::new();
        for p in fetched {
            let id_str = match p.id.and_then(|id| id.point_id_options) {
                Some(PointIdOptions::Num(n)) => n.to_string(),
                Some(PointIdOptions::Uuid(u)) => u,
                None => continue,
            };
            if let Some(eligibility) = lyrics_eligibility.as_ref()
                && !eligibility.matches(&id_str, &p.payload)
            {
                continue;
            }
            if let Some(vectors) = p.vectors
                && let Some(VectorsOptions::Vector(v)) = vectors.vectors_options
                && let VectorVariant::Dense(dense) = v.into_vector()
            {
                let data = dense.data;
                let cache_key = match lyrics_eligibility.as_ref() {
                    Some(eligibility) => eligibility.cache_key(&id_str),
                    None => Some(vec_cache_key(collection, &id_str)),
                };
                if let Some(cache_key) = cache_key {
                    to_cache.push((cache_key, vec_to_bytes(&data)));
                }
                out.insert(id_str, data);
            }
        }

        if !to_cache.is_empty()
            && let Ok(mut conn) = redis.get().await
        {
            let mut pipeline = deadpool_redis::redis::pipe();
            for (key, bytes) in to_cache {
                pipeline.set_ex::<_, _>(key, bytes, VEC_CACHE_TTL).ignore();
            }
            let _: Result<(), _> = pipeline.query_async::<()>(&mut conn).await;
        }
        out
    }
}

async fn load_lyrics_vector_eligibility(
    pool: &sqlx::PgPool,
    ids: &[String],
) -> Result<LyricsVectorEligibility, sqlx::Error> {
    if ids.is_empty() {
        return Ok(LyricsVectorEligibility::default());
    }
    let rows = sqlx::query_file!(
        "queries/recommendations/service/qdrant_io/lyrics_vector_eligibility.sql",
        ids
    )
    .fetch_all(pool)
    .await?;
    Ok(LyricsVectorEligibility {
        requests: rows
            .into_iter()
            .map(|row| (row.sc_track_id, row.request_id))
            .collect(),
    })
}

async fn load_public_track_ids(
    pool: &sqlx::PgPool,
    ids: &[String],
) -> Result<HashSet<String>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let rows = sqlx::query_file_scalar!(
        "queries/recommendations/service/qdrant_io/public_track_ids.sql",
        ids
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW_SEARCHES: [&str; 3] = ["search_points(", ".recommend(", "query_points("];
    const THE_FILTER: &str = "public_track_ids";

    fn serving_sources() -> Vec<(String, String)> {
        fn walk(directory: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(directory).expect("the source directory is readable") {
                let path = entry.expect("the directory entry is readable").path();
                if path.is_dir() {
                    walk(&path, found);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    found.push(path);
                }
            }
        }
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut paths = Vec::new();
        walk(&root.join("src"), &mut paths);
        paths.sort();
        paths
            .into_iter()
            .filter(|path| {
                !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains("test"))
            })
            .map(|path| {
                let shown = path
                    .strip_prefix(&root)
                    .expect("every source lives under the crate root")
                    .to_string_lossy()
                    .into_owned();
                let body = std::fs::read_to_string(&path).expect("every source is readable");
                (shown, body)
            })
            .collect()
    }

    #[test]
    fn whatever_qdrant_answers_is_checked_against_postgres_before_it_is_offered() {
        let mut searching = 0;
        for (path, body) in serving_sources() {
            if !RAW_SEARCHES.iter().any(|call| body.contains(call)) {
                continue;
            }
            searching += 1;
            assert!(
                body.contains(THE_FILTER),
                "{path} asks Qdrant for points and never asks PostgreSQL whether they may be \
                 offered; a vector outlives the track it belongs to, so this is how a private, \
                 deleted or unindexed track reaches a listener"
            );
        }
        assert!(
            searching >= 3,
            "only {searching} files search Qdrant; this guard is reading the wrong tree"
        );
    }

    fn payload(request_id: Option<&str>) -> HashMap<String, QdrantValue> {
        request_id
            .map(|request_id| {
                HashMap::from([(
                    LYRICS_VECTOR_REQUEST_FIELD.to_owned(),
                    QdrantValue {
                        kind: Some(QdrantValueKind::StringValue(request_id.to_owned())),
                    },
                )])
            })
            .unwrap_or_default()
    }

    #[test]
    fn only_a_point_embedded_for_the_current_request_is_served() {
        let eligibility = LyricsVectorEligibility {
            requests: HashMap::from([("42".to_owned(), "lyr:42:4".to_owned())]),
        };

        assert!(eligibility.matches("42", &payload(Some("lyr:42:4"))));
        assert!(
            !eligibility.matches("42", &payload(None)),
            "a point without a request is a bge-m3 vector from before the cutover"
        );
        assert!(
            !eligibility.matches("42", &payload(Some("lyr:42:3"))),
            "a point embedded for an older text is not the lyrics we hold now"
        );
        let malformed = HashMap::from([(
            LYRICS_VECTOR_REQUEST_FIELD.to_owned(),
            QdrantValue {
                kind: Some(QdrantValueKind::IntegerValue(4)),
            },
        )]);
        assert!(!eligibility.matches("42", &malformed));
        assert!(!eligibility.matches("43", &payload(Some("lyr:42:4"))));
        assert_eq!(
            eligibility.cache_key("42").as_deref(),
            Some("qv:lyrics:v4:42:lyr:42:4")
        );
        assert_eq!(eligibility.cache_key("43"), None);
    }

    async fn install_lyrics_catalog(pool: &sqlx::PgPool) -> anyhow::Result<()> {
        sqlx::raw_sql(
            "CREATE TABLE lyrics_cache (
                 sc_track_id text PRIMARY KEY,
                 embedded_at timestamptz,
                 embedding_state varchar(16),
                 created_at timestamp NOT NULL
             );
             CREATE TABLE lyrics_embedding_wire_state (
                 sc_track_id text PRIMARY KEY,
                 lyrics_created_at timestamp,
                 request_message_id varchar(128)
             );
             INSERT INTO lyrics_cache VALUES
                 ('1', now(), 'done', timestamp '2025-01-01'),
                 ('2', now(), 'legacy', timestamp '2025-01-02'),
                 ('3', now(), 'done', timestamp '2025-01-03'),
                 ('4', NULL, 'dispatched', timestamp '2025-01-04'),
                 ('5', now(), 'skipped', timestamp '2025-01-05'),
                 ('6', now(), 'done', timestamp '2025-01-06');
             INSERT INTO lyrics_embedding_wire_state VALUES
                 ('1', timestamp '2025-01-01', 'lyr:1:1'),
                 ('3', timestamp '2025-01-03', NULL),
                 ('4', timestamp '2025-01-04', 'lyr:4:1'),
                 ('5', timestamp '2025-01-05', 'lyr:5:1'),
                 ('6', timestamp '2025-01-01', 'lyr:6:1')",
        )
        .execute(pool)
        .await?;
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn postgres_allows_only_vectors_embedded_for_the_lyrics_it_holds(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        install_lyrics_catalog(&pool).await?;

        let ids = (1..=7).map(|id| id.to_string()).collect::<Vec<_>>();
        let eligibility = load_lyrics_vector_eligibility(&pool, &ids).await?;

        assert!(eligibility.matches("1", &payload(Some("lyr:1:1"))));
        assert!(!eligibility.contains("2"), "legacy rows are bge-m3 vectors");
        assert!(!eligibility.contains("3"), "done without a request id");
        assert!(!eligibility.contains("4"), "still waiting for the worker");
        assert!(!eligibility.contains("5"));
        assert!(
            !eligibility.contains("6"),
            "the request was made for lyrics that have since been replaced"
        );
        assert!(!eligibility.contains("7"));

        sqlx::query(
            "UPDATE lyrics_cache SET embedding_state = 'quarantined' WHERE sc_track_id = '1'",
        )
        .execute(&pool)
        .await?;
        let eligibility = load_lyrics_vector_eligibility(&pool, &ids).await?;
        assert!(!eligibility.allows_all(&["1".to_owned()]));
        Ok(())
    }

    #[sqlx::test(migrations = false)]
    async fn public_cache_snapshot_is_invalidated_when_track_becomes_private(
        pool: sqlx::PgPool,
    ) -> anyhow::Result<()> {
        sqlx::raw_sql(
            "CREATE TABLE tracks (
                 sc_track_id text PRIMARY KEY,
                 sharing text NOT NULL,
                 index_state text NOT NULL,
                 storage_state text NOT NULL,
                 needs_duration_resolve boolean NOT NULL,
                 superseded_by uuid
             );
             INSERT INTO tracks (sc_track_id, sharing, index_state, storage_state, needs_duration_resolve)
             VALUES ('1', 'public', 'indexed', 'ok', false)",
        )
        .execute(&pool)
        .await?;
        let ids = vec!["1".to_owned()];

        let public = load_public_track_ids(&pool, &ids).await?;
        assert!(ids.iter().all(|track_id| public.contains(track_id)));

        sqlx::query("UPDATE tracks SET sharing = 'private' WHERE sc_track_id = '1'")
            .execute(&pool)
            .await?;
        let public = load_public_track_ids(&pool, &ids).await?;
        assert!(!ids.iter().all(|track_id| public.contains(track_id)));
        Ok(())
    }
}

#[cfg(test)]
#[path = "qdrant_io_live_tests.rs"]
mod live_tests;
