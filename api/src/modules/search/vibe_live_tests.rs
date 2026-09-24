use std::collections::HashMap;
use std::sync::Arc;

use backend_contracts::vector_store::{QUERY_VEC_MULAN, query_point_uuid};
use qdrant_client::qdrant::{
    CreateCollectionBuilder, Distance, PointStruct, UpsertPointsBuilder, VectorParamsBuilder,
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::cache::CacheService;
use crate::modules::lyrics::WorkerClient;
use crate::modules::recommendations::RecommendationsService;
use crate::modules::recommendations::live_fixture::{
    ARTISTS, TrackSeed, catalogue, genre_of, install_all_vectors, install_catalog, pointing_in,
    service,
};

const MULAN_DIMENSIONS: u64 = 512;

use super::semantic::VibeSearchService;
use super::vibe::VibeResponse;

const LIMIT: usize = 24;
const ARTIST_CAP: usize = 3;

async fn vibe(pg: PgPool) -> anyhow::Result<(Arc<VibeSearchService>, Arc<RecommendationsService>)> {
    let recommendations = service(pg.clone()).await?;
    let redis = deadpool_redis::Config::from_url(
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned()),
    )
    .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let cache = CacheService::new(redis);
    let qdrant = recommendations.qdrant.clone();
    let worker = WorkerClient::new(recommendations.nats.clone(), cache.clone(), qdrant.clone());
    let vibe = VibeSearchService::new(pg, cache, recommendations.clone(), worker, qdrant);
    Ok((vibe, recommendations))
}

async fn install_query_vector(
    service: &RecommendationsService,
    query: &str,
    vector: Vec<f32>,
) -> anyhow::Result<()> {
    let hash = hex::encode(Sha256::digest(query.trim().as_bytes()));
    let client = service.qdrant.raw();
    let _ = client.delete_collection(QUERY_VEC_MULAN).await;
    client
        .create_collection(
            CreateCollectionBuilder::new(QUERY_VEC_MULAN)
                .vectors_config(VectorParamsBuilder::new(MULAN_DIMENSIONS, Distance::Cosine)),
        )
        .await?;
    client
        .upsert_points(
            UpsertPointsBuilder::new(
                QUERY_VEC_MULAN,
                vec![PointStruct::new(
                    query_point_uuid(&hash),
                    vector,
                    qdrant_client::Payload::from(serde_json::Map::from_iter([(
                        crate::qdrant::QUERY_VECTOR_ENCODER_FIELD.to_owned(),
                        serde_json::json!("OpenMuQ/MuQ-MuLan-large@2e01c796"),
                    )])),
                )],
            )
            .wait(true),
        )
        .await?;
    use deadpool_redis::redis::AsyncCommands;
    let mut connection = service.redis.get().await?;
    let _: () = connection.del(format!("vibe:vec:mulan:v1:{hash}")).await?;
    Ok(())
}

async fn install_everything(
    recommendations: &RecommendationsService,
    tracks: &[TrackSeed],
    query: &str,
) -> anyhow::Result<()> {
    install_all_vectors(recommendations, tracks).await?;
    install_query_vector(recommendations, query, pointing_in(0, MULAN_DIMENSIONS)).await
}

fn fresh(base: &str) -> String {
    format!("{base} {}", std::process::id())
}

async fn ask(vibe: &VibeSearchService, query: &str) -> anyhow::Result<VibeResponse> {
    Ok(Box::pin(vibe.vibe(query, Some(LIMIT), None)).await?)
}

fn ids(page: &VibeResponse) -> Vec<String> {
    page.items
        .iter()
        .filter_map(|item| {
            item.get("id")
                .and_then(|id| id.as_u64().map(|id| id.to_string()))
        })
        .collect()
}

fn uploaders(page: &VibeResponse) -> Vec<String> {
    page.items
        .iter()
        .filter_map(|item| {
            item.get("user")
                .and_then(|user| user.get("username"))
                .and_then(|name| name.as_str())
                .map(str::to_owned)
        })
        .collect()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn a_vibe_we_already_have_a_vector_for_is_answered_without_the_worker(
    pg: PgPool,
) -> anyhow::Result<()> {
    let query = fresh("something warm and slow for the evening");
    let tracks = catalogue(660_000, 60);
    install_catalog(&pg, &tracks).await?;

    let (vibe, recommendations) = vibe(pg).await?;
    install_everything(&recommendations, &tracks, &query).await?;

    let page = ask(&vibe, &query).await?;

    assert_eq!(
        page.status, "ready",
        "the vector for this query is already in Qdrant, so the answer is ready, not pending"
    );
    assert!(
        !page.items.is_empty(),
        "a ready answer with nothing in it is the same as no answer: {page:?}"
    );
    let known: std::collections::HashSet<String> = tracks
        .iter()
        .map(|track| track.sc_track_id.to_string())
        .collect();
    assert!(
        ids(&page).iter().all(|id| known.contains(id)),
        "the answer holds a track that is not in the catalogue: {:?}",
        ids(&page)
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn no_single_uploader_is_allowed_to_fill_the_page(pg: PgPool) -> anyhow::Result<()> {
    let query = fresh("something one artist made all of");
    let tracks = catalogue(670_000, 60);
    install_catalog(&pg, &tracks).await?;

    let (vibe, recommendations) = vibe(pg).await?;
    install_everything(&recommendations, &tracks, &query).await?;

    let page = ask(&vibe, &query).await?;
    let mut per_uploader: HashMap<String, usize> = HashMap::new();
    for name in uploaders(&page) {
        *per_uploader.entry(name).or_default() += 1;
    }

    assert!(
        !per_uploader.is_empty(),
        "no uploader made it into the answer, so the cap cannot be observed: {page:?}"
    );
    for (name, count) in &per_uploader {
        assert!(
            *count <= ARTIST_CAP,
            "`{name}` takes {count} of the page while the cap is {ARTIST_CAP}: {per_uploader:?}"
        );
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn a_track_postgres_refuses_is_absent_from_the_atmosphere(pg: PgPool) -> anyhow::Result<()> {
    let query = fresh("something the catalogue will take back");
    let tracks = catalogue(680_000, 60);
    install_catalog(&pg, &tracks).await?;

    let (vibe, recommendations) = vibe(pg.clone()).await?;
    install_everything(&recommendations, &tracks, &query).await?;

    let before = ids(&ask(&vibe, &query).await?);
    let doomed = before.first().cloned().expect("the search found something");
    sqlx::query("UPDATE tracks SET index_state = 'pending' WHERE sc_track_id = $1")
        .bind(&doomed)
        .execute(&pg)
        .await?;

    let after = ids(&ask(&vibe, &query).await?);

    assert!(
        !after.contains(&doomed),
        "{doomed} lost its index and still answers the same search, cache and all: {after:?}"
    );
    assert!(
        !after.is_empty(),
        "the whole answer disappeared, so this test would pass for the wrong reason"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn the_atmosphere_names_only_genres_that_are_actually_on_the_page(
    pg: PgPool,
) -> anyhow::Result<()> {
    let query = fresh("something with a genre behind it");
    let tracks = catalogue(690_000, 60);
    install_catalog(&pg, &tracks).await?;

    let (vibe, recommendations) = vibe(pg).await?;
    install_everything(&recommendations, &tracks, &query).await?;

    let page = ask(&vibe, &query).await?;
    let shown: std::collections::HashSet<String> = uploaders(&page)
        .into_iter()
        .map(|name| genre_of(&name).to_owned())
        .collect();

    assert!(
        !page.atmosphere.top_genres.is_empty(),
        "the page has tracks but names no genre at all: {page:?}"
    );
    for genre in &page.atmosphere.top_genres {
        assert!(
            shown.contains(genre),
            "`{genre}` is named as the atmosphere of a page that holds none of it: {shown:?}"
        );
    }
    assert!(
        ARTISTS
            .iter()
            .any(|artist| shown.contains(genre_of(artist))),
        "the fixture gave every uploader a genre, so at least one must show: {shown:?}"
    );
    Ok(())
}
