use std::sync::Arc;

use qdrant_client::Payload;
use qdrant_client::qdrant::{
    CreateCollectionBuilder, Distance, PointStruct, UpsertPointsBuilder, VectorParamsBuilder,
};
use serde_json::json;
use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::bus::nats::NatsService;
use crate::cache::CacheService;
use crate::config::{QdrantCfg, SoundwaveCfg};
use crate::modules::collab::CollabVectorService;
use crate::modules::lyrics::WorkerClient;
use crate::qdrant::{QdrantService, collections};

use super::clusters::ClusterResponse;
use super::s3_verifier::S3VerifierService;
use super::service::{LYRICS_VECTOR_REQUEST_FIELD, RecommendationsService, lyrics_vec_cache_key};

pub const LISTENER: &str = "soundcloud:users:770001";
pub const TWIN: &str = "soundcloud:users:770002";
pub const SPREAD: usize = 5;

pub fn dimensions_of(collection: &str) -> u64 {
    match collection {
        collections::TRACKS_CLAP => 512,
        collections::TRACKS_COLLAB => 128,
        _ => 1024,
    }
}
pub const PER_CLUSTER: usize = 6;
pub const LIKES: u64 = 10;
pub const VECTOR_CLUSTERS: [&str; 3] = ["wave", "same_vibe", "deep_cuts"];
pub const ARTISTS: [&str; 4] = ["Aster", "Bellwether", "Cinder", "Dovetail"];
pub const GENRES: [&str; 4] = ["ambient", "breakbeat", "chillwave", "dub"];

pub fn genre_of(artist: &str) -> &'static str {
    let index = ARTISTS
        .iter()
        .position(|name| *name == artist)
        .unwrap_or_default();
    GENRES[index]
}

pub fn uploader_id(artist: &str) -> String {
    let index = ARTISTS
        .iter()
        .position(|name| *name == artist)
        .unwrap_or_default();
    format!("soundcloud:users:88000{index}")
}

fn qdrant_url() -> String {
    std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://127.0.0.1:6334".to_owned())
}

fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned())
}

fn nats_url() -> String {
    std::env::var("NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4224".to_owned())
}

pub async fn service(pg: PgPool) -> anyhow::Result<Arc<RecommendationsService>> {
    let qdrant = QdrantService::connect(&QdrantCfg {
        grpc_url: qdrant_url(),
        api_key: String::new().into(),
    })?;
    let redis = deadpool_redis::Config::from_url(redis_url())
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let nats = NatsService::connect(&nats_url(), CancellationToken::new()).await?;
    let cache = CacheService::new(redis.clone());
    let worker = WorkerClient::new(nats.clone(), cache, qdrant.clone());
    let s3 = S3VerifierService::new(wreq::Client::new(), String::new(), pg.clone());
    let collab = CollabVectorService::new(qdrant.clone());
    Ok(RecommendationsService::new(
        qdrant,
        pg,
        nats,
        redis,
        worker,
        s3,
        collab,
        SoundwaveCfg {
            popularity_boost: 0.0,
            artist_cap: 3,
        },
    ))
}

pub struct TrackSeed {
    pub sc_track_id: u64,
    pub artist: &'static str,
    pub plays: i64,
}

pub fn seed(index: u64, first_id: u64) -> TrackSeed {
    TrackSeed {
        sc_track_id: first_id + index,
        artist: ARTISTS[index as usize % ARTISTS.len()],
        plays: 5_000,
    }
}

pub fn catalogue(first_id: u64, count: u64) -> Vec<TrackSeed> {
    (0..count).map(|index| seed(index, first_id)).collect()
}

pub fn by_one_artist(first_id: u64, count: u64) -> Vec<TrackSeed> {
    (0..count)
        .map(|index| TrackSeed {
            artist: ARTISTS[0],
            ..seed(index, first_id)
        })
        .collect()
}

pub fn lyrics_request_id(sc_track_id: &str) -> String {
    format!("lyr:{sc_track_id}:1")
}

fn point_payload(collection: &str, sc_track_id: &str) -> Payload {
    let mut payload = serde_json::Map::from_iter([("sc_track_id".to_owned(), json!(sc_track_id))]);
    if collection == collections::TRACKS_LYRICS {
        payload.insert(
            LYRICS_VECTOR_REQUEST_FIELD.to_owned(),
            json!(lyrics_request_id(sc_track_id)),
        );
    }
    Payload::from(payload)
}

pub fn pointing_in(at: usize, dimensions: u64) -> Vec<f32> {
    let width = dimensions as usize;
    let mut vector = vec![0.01_f32; width];
    vector[at % SPREAD.min(width)] = 1.0;
    vector[(at + 1) % SPREAD.min(width)] = 0.5 - (at as f32) * 0.001;
    vector
}

pub async fn install_catalog(pg: &PgPool, tracks: &[TrackSeed]) -> anyhow::Result<()> {
    for name in ARTISTS {
        sqlx::query(
            "INSERT INTO artists (id, name, normalized_name, source, track_count_primary)
             VALUES ($1, $2, lower($2), 'test', 1)",
        )
        .bind(Uuid::now_v7())
        .bind(name)
        .execute(pg)
        .await?;
    }
    for track in tracks {
        let artist_id: Uuid = sqlx::query_scalar("SELECT id FROM artists WHERE name = $1")
            .bind(track.artist)
            .fetch_one(pg)
            .await?;
        let track_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO tracks (
                 id, sc_track_id, urn, title, title_normalized, duration_ms,
                 sharing, storage_state, index_state, needs_duration_resolve,
                 primary_artist_id, uploader_sc_user_id, uploader_username, genre,
                 sc_synced_at, sc_created_at, language
             ) VALUES (
                 $1, $2, 'soundcloud:tracks:' || $2, $3, lower($3), 210000,
                 'public', 'ok', 'indexed', false,
                 $4, $5, $6, $7,
                 now() - interval '1 day', now() - interval '2 days', 'en'
             )",
        )
        .bind(track_id)
        .bind(track.sc_track_id.to_string())
        .bind(format!("Song {}", track.sc_track_id))
        .bind(artist_id)
        .bind(uploader_id(track.artist))
        .bind(track.artist)
        .bind(genre_of(track.artist))
        .execute(pg)
        .await?;
        sqlx::query(
            "INSERT INTO track_artists (track_id, artist_id, role, source)
             VALUES ($1, $2, 'primary', 'test')",
        )
        .bind(track_id)
        .bind(artist_id)
        .execute(pg)
        .await?;
        sqlx::query(
            "INSERT INTO sc_track_counters (sc_track_id, play_count, fetched_at)
             VALUES ($1, $2, now())",
        )
        .bind(track.sc_track_id.to_string())
        .bind(track.plays)
        .execute(pg)
        .await?;
    }
    Ok(())
}

pub async fn like(pg: &PgPool, listener: &str, sc_track_id: u64) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO user_events (id, sc_user_id, sc_track_id, event_type, weight, created_at)
         VALUES (gen_random_uuid(), $1, $2, 'like', 1.0, now() - interval '1 hour')",
    )
    .bind(listener)
    .bind(sc_track_id.to_string())
    .execute(pg)
    .await?;
    sqlx::query(
        "INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state, created_at)
         VALUES ($1, $2, true, now() - interval '1 hour')",
    )
    .bind(listener)
    .bind(sc_track_id.to_string())
    .execute(pg)
    .await?;
    Ok(())
}

pub async fn likes_the_same_ten(pg: &PgPool, listener: &str, first_id: u64) -> anyhow::Result<()> {
    for index in 0..LIKES {
        like(pg, listener, first_id + index).await?;
    }
    Ok(())
}

pub async fn dislike(pg: &PgPool, listener: &str, sc_track_id: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO disliked_tracks (id, sc_user_id, sc_track_id, created_at)
         VALUES (gen_random_uuid(), $1, $2, now())",
    )
    .bind(listener)
    .bind(sc_track_id)
    .execute(pg)
    .await?;
    Ok(())
}

pub async fn install_vectors(
    service: &RecommendationsService,
    collection: &str,
    tracks: &[TrackSeed],
) -> anyhow::Result<()> {
    let dimensions = dimensions_of(collection);
    let client = service.qdrant.raw();
    let _ = client.delete_collection(collection).await;
    client
        .create_collection(
            CreateCollectionBuilder::new(collection)
                .vectors_config(VectorParamsBuilder::new(dimensions, Distance::Cosine)),
        )
        .await?;
    if tracks.is_empty() {
        return Ok(());
    }
    let points: Vec<PointStruct> = tracks
        .iter()
        .enumerate()
        .map(|(index, track)| {
            PointStruct::new(
                track.sc_track_id,
                pointing_in(index, dimensions),
                point_payload(collection, &track.sc_track_id.to_string()),
            )
        })
        .collect();
    client
        .upsert_points(UpsertPointsBuilder::new(collection, points).wait(true))
        .await?;
    forget_cached_vectors(service, collection, tracks).await?;
    Ok(())
}

async fn forget_cached_vectors(
    service: &RecommendationsService,
    collection: &str,
    tracks: &[TrackSeed],
) -> anyhow::Result<()> {
    use deadpool_redis::redis::AsyncCommands;
    let mut connection = service.redis.get().await?;
    for track in tracks {
        let id = track.sc_track_id.to_string();
        let key = if collection == collections::TRACKS_LYRICS {
            lyrics_vec_cache_key(&id, &lyrics_request_id(&id))
        } else {
            format!("qv:{collection}:{id}")
        };
        let _: () = connection.del(key).await?;
    }
    Ok(())
}

pub async fn install_all_vectors(
    service: &RecommendationsService,
    tracks: &[TrackSeed],
) -> anyhow::Result<()> {
    for collection in [
        collections::TRACKS_MERT,
        collections::TRACKS_CLAP,
        collections::TRACKS_COLLAB,
    ] {
        install_vectors(service, collection, tracks).await?;
    }
    install_vectors(service, collections::TRACKS_LYRICS, &[]).await?;
    Ok(())
}

pub fn offered_ids(response: &ClusterResponse) -> Vec<String> {
    response
        .clusters
        .iter()
        .flat_map(|cluster| cluster.track_ids.iter().cloned())
        .collect()
}

pub fn offered_by_vectors(response: &ClusterResponse) -> Vec<String> {
    response
        .clusters
        .iter()
        .filter(|cluster| VECTOR_CLUSTERS.contains(&cluster.id))
        .flat_map(|cluster| cluster.track_ids.iter().cloned())
        .collect()
}

pub fn cluster_names(response: &ClusterResponse) -> Vec<&'static str> {
    response.clusters.iter().map(|cluster| cluster.id).collect()
}

pub fn in_cluster(response: &ClusterResponse, id: &str) -> Vec<&'static str> {
    response
        .clusters
        .iter()
        .filter(|cluster| cluster.track_ids.iter().any(|track| track == id))
        .map(|cluster| cluster.id)
        .collect()
}

pub fn assert_the_vector_clusters_were_built(response: &ClusterResponse) {
    let names = cluster_names(response);
    for expected in VECTOR_CLUSTERS {
        assert!(
            names.contains(&expected),
            "`{expected}` is missing, so nothing in this wave came from Qdrant and the test \
             would pass even with the vector path broken: {names:?}"
        );
    }
}
