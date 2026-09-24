use qdrant_client::Payload;
use qdrant_client::qdrant::{
    CreateCollectionBuilder, Distance, PointStruct, UpsertPointsBuilder, VectorParamsBuilder,
};
use serde_json::json;
use sqlx::PgPool;

use super::clusters::ClusterResponse;
use super::home_wave::HomeRequest;
use super::live_fixture::{
    LISTENER, PER_CLUSTER, TWIN, TrackSeed, assert_the_vector_clusters_were_built, catalogue,
    cluster_names, install_all_vectors, install_catalog, likes_the_same_ten, service,
};
use super::service::RecommendationsService;

const FIRST_TRACK: u64 = 890_000;
const VERSION: &str = "taste-202609241200-0000beef";
const COLLECTION: &str = "tracks_taste_202609241200_0000beef";
const DIMENSIONS: usize = 128;
const LOVED_INDEX: usize = 40;

async fn home_wave(
    service: &RecommendationsService,
    listener: &str,
) -> anyhow::Result<ClusterResponse> {
    Ok(Box::pin(service.home_wave(HomeRequest {
        sc_user_id: listener.to_owned(),
        languages: None,
        per_cluster: PER_CLUSTER,
        hide_listened: false,
    }))
    .await?)
}

fn towards(index: usize) -> Vec<f32> {
    let mut vector = vec![0.01_f32; DIMENSIONS];
    if let Some(slot) = vector.get_mut(index % DIMENSIONS) {
        *slot = 1.0;
    }
    vector
}

async fn install_taste_space(
    service: &RecommendationsService,
    tracks: &[TrackSeed],
) -> anyhow::Result<()> {
    let client = service.qdrant.raw();
    let _ = client.delete_collection(COLLECTION).await;
    client
        .create_collection(CreateCollectionBuilder::new(COLLECTION).vectors_config(
            VectorParamsBuilder::new(DIMENSIONS as u64, Distance::Cosine),
        ))
        .await?;
    let points = tracks
        .iter()
        .enumerate()
        .map(|(index, track)| {
            let payload: Payload = json!({ "sc_track_id": track.sc_track_id.to_string() })
                .try_into()
                .map_err(|error| anyhow::anyhow!("{error:?}"))?;
            Ok(PointStruct::new(track.sc_track_id, towards(index), payload))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    client
        .upsert_points(UpsertPointsBuilder::new(COLLECTION, points).wait(true))
        .await?;
    Ok(())
}

async fn serve_version(pg: &PgPool, listener: &str) -> anyhow::Result<()> {
    serve_version_towards(pg, listener, LOVED_INDEX).await
}

async fn serve_version_towards(pg: &PgPool, listener: &str, index: usize) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO taste_model_versions
             (version, input_object, collection, trained_at, dim, pooling, metrics,
              items_count, users_count, active, applied_at)
         VALUES ($1, 'taste-input-live', $2, now(), 128, '{}', '{}', 60, 1, true, now())",
    )
    .bind(VERSION)
    .bind(COLLECTION)
    .execute(pg)
    .await?;
    let account = listener.rsplit(':').next().unwrap_or(listener);
    sqlx::query("INSERT INTO user_taste_vectors (sc_user_id, version, vec) VALUES ($1, $2, $3)")
        .bind(account)
        .bind(VERSION)
        .bind(towards(index))
        .execute(pg)
        .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn the_taste_shelf_never_offers_what_the_listener_already_likes(
    pg: PgPool,
) -> anyhow::Result<()> {
    let tracks = catalogue(FIRST_TRACK, 60);
    install_catalog(&pg, &tracks).await?;
    likes_the_same_ten(&pg, LISTENER, FIRST_TRACK).await?;
    sqlx::query(
        "INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state, created_at)
         VALUES ($1, $2, true, now() - interval '300 days')",
    )
    .bind(LISTENER)
    .bind((FIRST_TRACK + 11).to_string())
    .execute(&pg)
    .await?;
    let service = service(pg.clone()).await?;
    install_all_vectors(&service, &tracks).await?;
    install_taste_space(&service, &tracks).await?;
    serve_version_towards(&pg, LISTENER, 3).await?;

    let response = home_wave(&service, LISTENER).await?;
    service.qdrant.raw().delete_collection(COLLECTION).await?;

    let taste = shelf(&response, "taste");
    let owned: Vec<String> = (0..12)
        .filter(|index| *index != 10)
        .map(|index| (FIRST_TRACK + index).to_string())
        .collect();
    assert!(
        !taste.is_empty(),
        "the shelf still fills: {:?}",
        cluster_names(&response)
    );
    assert!(
        taste.iter().all(|id| !owned.iter().any(|mine| mine == id)),
        "liked and imported tracks stay off the taste shelf: {taste:?}"
    );
    Ok(())
}

fn shelf<'a>(response: &'a ClusterResponse, id: &str) -> Vec<&'a str> {
    response
        .clusters
        .iter()
        .filter(|cluster| cluster.id == id)
        .flat_map(|cluster| cluster.track_ids.iter().map(String::as_str))
        .collect()
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn a_listener_with_a_taste_vector_gets_a_taste_shelf_and_the_rest_of_the_wave_stays(
    pg: PgPool,
) -> anyhow::Result<()> {
    let tracks = catalogue(FIRST_TRACK, 60);
    install_catalog(&pg, &tracks).await?;
    likes_the_same_ten(&pg, LISTENER, FIRST_TRACK).await?;
    likes_the_same_ten(&pg, TWIN, FIRST_TRACK).await?;
    let service = service(pg.clone()).await?;
    install_all_vectors(&service, &tracks).await?;
    install_taste_space(&service, &tracks).await?;
    serve_version(&pg, LISTENER).await?;

    let with_taste = home_wave(&service, LISTENER).await?;
    let without = home_wave(&service, TWIN).await?;
    sqlx::query("UPDATE taste_model_versions SET active = false")
        .execute(&pg)
        .await?;
    let retired = home_wave(&service, LISTENER).await?;
    service.qdrant.raw().delete_collection(COLLECTION).await?;

    let taste = shelf(&with_taste, "taste");
    let loved = (FIRST_TRACK + LOVED_INDEX as u64).to_string();
    assert!(
        !taste.is_empty(),
        "a listener the model knows gets its shelf: {:?}",
        cluster_names(&with_taste)
    );
    assert!(
        taste.contains(&loved.as_str()),
        "the track nearest to the taste vector leads the shelf: {taste:?}"
    );
    assert_the_vector_clusters_were_built(&with_taste);
    assert_the_vector_clusters_were_built(&without);
    assert!(
        !cluster_names(&without).contains(&"taste"),
        "a listener without a vector keeps the wave it had before the model existed"
    );
    assert!(
        !cluster_names(&retired).contains(&"taste"),
        "a vector of a retired model is never searched"
    );
    Ok(())
}
