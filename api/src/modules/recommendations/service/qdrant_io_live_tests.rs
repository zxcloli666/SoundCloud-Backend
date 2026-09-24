use std::collections::HashMap;
use std::sync::Arc;

use backend_contracts::vector_store::TRACKS_LYRICS_DIMENSIONS;
use deadpool_redis::redis::AsyncCommands;
use qdrant_client::Payload;
use qdrant_client::qdrant::{
    CreateCollectionBuilder, Distance, PointStruct, UpsertPointsBuilder, VectorParamsBuilder,
};
use serde_json::json;
use sqlx::PgPool;

use super::retrieve_vectors_with;
use crate::config::QdrantCfg;
use crate::qdrant::QdrantService;
use crate::qdrant::collections::TRACKS_LYRICS;

const UNMARKED_POINT: u64 = 9_000_001;
const MARKED_POINT: u64 = 9_000_002;
const VECTOR_SIZE: u64 = 4;
const CURRENT_REQUEST: &str = "epoch-2";
const LYRICS_CREATED_AT: &str = "2025-01-01 00:00:00";

fn qdrant_url() -> String {
    std::env::var("QDRANT_URL").unwrap_or_else(|_| "http://127.0.0.1:6334".to_owned())
}

fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned())
}

fn collection_name() -> String {
    TRACKS_LYRICS.to_owned()
}

async fn restore_collection(service: &QdrantService) -> anyhow::Result<()> {
    let _ = service.raw().delete_collection(TRACKS_LYRICS).await;
    service
        .raw()
        .create_collection(CreateCollectionBuilder::new(TRACKS_LYRICS).vectors_config(
            VectorParamsBuilder::new(TRACKS_LYRICS_DIMENSIONS, Distance::Cosine),
        ))
        .await?;
    Ok(())
}

async fn qdrant() -> anyhow::Result<Arc<QdrantService>> {
    Ok(QdrantService::connect(&QdrantCfg {
        grpc_url: qdrant_url(),
        api_key: String::new().into(),
    })?)
}

fn redis() -> anyhow::Result<deadpool_redis::Pool> {
    Ok(deadpool_redis::Config::from_url(redis_url())
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?)
}

async fn seed_collection(
    service: &QdrantService,
    collection: &str,
    points: Vec<PointStruct>,
) -> anyhow::Result<()> {
    let _ = service.raw().delete_collection(collection).await;
    service
        .raw()
        .create_collection(
            CreateCollectionBuilder::new(collection)
                .vectors_config(VectorParamsBuilder::new(VECTOR_SIZE, Distance::Cosine)),
        )
        .await?;
    service
        .raw()
        .upsert_points(UpsertPointsBuilder::new(collection, points).wait(true))
        .await?;
    Ok(())
}

fn unmarked_point(id: u64, vector: Vec<f32>) -> PointStruct {
    PointStruct::new(id, vector, Payload::new())
}

fn marked_point(id: u64, request_id: &str, vector: Vec<f32>) -> PointStruct {
    let payload: Payload = json!({ "embedding_request_id": request_id })
        .try_into()
        .expect("payload");
    PointStruct::new(id, vector, payload)
}

async fn install_catalog(pool: &PgPool, embedded: &[u64]) -> anyhow::Result<()> {
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
         )",
    )
    .execute(pool)
    .await?;
    for id in embedded {
        sqlx::query("INSERT INTO lyrics_cache VALUES ($1, now(), 'done', $2::timestamp)")
            .bind(id.to_string())
            .bind(LYRICS_CREATED_AT)
            .execute(pool)
            .await?;
        sqlx::query("INSERT INTO lyrics_embedding_wire_state VALUES ($1, $2::timestamp, $3)")
            .bind(id.to_string())
            .bind(LYRICS_CREATED_AT)
            .bind(CURRENT_REQUEST)
            .execute(pool)
            .await?;
    }
    Ok(())
}

fn cache_key(id: u64) -> String {
    format!("qv:lyrics:v4:{id}:{CURRENT_REQUEST}")
}

async fn clear_cache(redis: &deadpool_redis::Pool, ids: &[u64]) -> anyhow::Result<()> {
    let mut conn = redis.get().await?;
    for id in ids {
        let _: () = conn.del(cache_key(*id)).await?;
    }
    Ok(())
}

async fn cached_key_exists(redis: &deadpool_redis::Pool, id: u64) -> anyhow::Result<bool> {
    let mut conn = redis.get().await?;
    Ok(conn.exists(cache_key(id)).await?)
}

async fn await_cached(redis: &deadpool_redis::Pool, id: u64) -> anyhow::Result<()> {
    for _ in 0..100 {
        if cached_key_exists(redis, id).await? {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    Err(anyhow::anyhow!("vector was never cached"))
}

#[sqlx::test(migrations = false)]
#[ignore = "requires a local Qdrant and Redis"]
async fn an_unmarked_point_is_refused_while_a_point_marked_with_the_request_is_served(
    pool: PgPool,
) -> anyhow::Result<()> {
    let service = qdrant().await?;
    let redis = redis()?;
    let collection = collection_name();
    install_catalog(&pool, &[UNMARKED_POINT, MARKED_POINT]).await?;
    clear_cache(&redis, &[UNMARKED_POINT, MARKED_POINT]).await?;
    seed_collection(
        &service,
        &collection,
        vec![
            unmarked_point(UNMARKED_POINT, vec![1.0, 0.0, 0.0, 0.0]),
            marked_point(MARKED_POINT, CURRENT_REQUEST, vec![0.0, 1.0, 0.0, 0.0]),
        ],
    )
    .await?;

    let vectors = retrieve_vectors_with(
        &service,
        &redis,
        &pool,
        &collection,
        &[UNMARKED_POINT, MARKED_POINT],
    )
    .await;

    assert_eq!(
        vectors.keys().cloned().collect::<Vec<_>>(),
        vec![MARKED_POINT.to_string()]
    );
    assert!(!cached_key_exists(&redis, UNMARKED_POINT).await?);
    restore_collection(&service).await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
#[ignore = "requires a local Qdrant and Redis"]
async fn a_cache_miss_populates_the_cache_and_a_hit_serves_without_qdrant(
    pool: PgPool,
) -> anyhow::Result<()> {
    let service = qdrant().await?;
    let redis = redis()?;
    let collection = collection_name();
    install_catalog(&pool, &[MARKED_POINT]).await?;
    clear_cache(&redis, &[MARKED_POINT]).await?;
    seed_collection(
        &service,
        &collection,
        vec![marked_point(
            MARKED_POINT,
            CURRENT_REQUEST,
            vec![0.5, 0.5, 0.5, 0.5],
        )],
    )
    .await?;

    let miss = retrieve_vectors_with(&service, &redis, &pool, &collection, &[MARKED_POINT]).await;
    assert_eq!(miss.len(), 1);
    await_cached(&redis, MARKED_POINT).await?;

    service.raw().delete_collection(&collection).await?;
    let hit = retrieve_vectors_with(&service, &redis, &pool, &collection, &[MARKED_POINT]).await;

    assert_eq!(hit, miss);
    restore_collection(&service).await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
#[ignore = "requires a local Qdrant and Redis"]
async fn a_point_overwritten_for_another_request_stops_being_served(
    pool: PgPool,
) -> anyhow::Result<()> {
    let service = qdrant().await?;
    let redis = redis()?;
    let collection = collection_name();
    install_catalog(&pool, &[MARKED_POINT]).await?;
    clear_cache(&redis, &[MARKED_POINT]).await?;
    seed_collection(
        &service,
        &collection,
        vec![marked_point(
            MARKED_POINT,
            CURRENT_REQUEST,
            vec![1.0, 0.0, 0.0, 0.0],
        )],
    )
    .await?;
    assert_eq!(
        retrieve_vectors_with(&service, &redis, &pool, &collection, &[MARKED_POINT])
            .await
            .len(),
        1
    );
    await_cached(&redis, MARKED_POINT).await?;

    clear_cache(&redis, &[MARKED_POINT]).await?;
    service
        .raw()
        .upsert_points(
            UpsertPointsBuilder::new(
                &collection,
                vec![marked_point(
                    MARKED_POINT,
                    "epoch-3",
                    vec![0.0, 0.0, 1.0, 0.0],
                )],
            )
            .wait(true),
        )
        .await?;

    let after = retrieve_vectors_with(&service, &redis, &pool, &collection, &[MARKED_POINT]).await;

    assert!(after.is_empty());
    assert!(!cached_key_exists(&redis, MARKED_POINT).await?);
    restore_collection(&service).await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
#[ignore = "requires a local Qdrant and Redis"]
async fn postgres_leaving_the_done_state_refuses_the_point_before_qdrant_is_touched(
    pool: PgPool,
) -> anyhow::Result<()> {
    let service = qdrant().await?;
    let redis = redis()?;
    let collection = collection_name();
    install_catalog(&pool, &[MARKED_POINT]).await?;
    clear_cache(&redis, &[MARKED_POINT]).await?;
    seed_collection(
        &service,
        &collection,
        vec![marked_point(
            MARKED_POINT,
            CURRENT_REQUEST,
            vec![1.0, 0.0, 0.0, 0.0],
        )],
    )
    .await?;

    sqlx::query("UPDATE lyrics_cache SET embedding_state = 'quarantined'")
        .execute(&pool)
        .await?;
    let refused: HashMap<String, Vec<f32>> =
        retrieve_vectors_with(&service, &redis, &pool, &collection, &[MARKED_POINT]).await;

    assert!(refused.is_empty());
    restore_collection(&service).await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
#[ignore = "requires a local Qdrant and Redis"]
async fn postgres_leaving_the_done_state_closes_the_cache_path_too(
    pool: PgPool,
) -> anyhow::Result<()> {
    let service = qdrant().await?;
    let redis = redis()?;
    let collection = collection_name();
    install_catalog(&pool, &[MARKED_POINT]).await?;
    clear_cache(&redis, &[MARKED_POINT]).await?;
    seed_collection(
        &service,
        &collection,
        vec![marked_point(
            MARKED_POINT,
            CURRENT_REQUEST,
            vec![1.0, 0.0, 0.0, 0.0],
        )],
    )
    .await?;
    assert_eq!(
        retrieve_vectors_with(&service, &redis, &pool, &collection, &[MARKED_POINT])
            .await
            .len(),
        1
    );
    await_cached(&redis, MARKED_POINT).await?;

    sqlx::query("UPDATE lyrics_cache SET embedding_state = 'quarantined'")
        .execute(&pool)
        .await?;
    let after = retrieve_vectors_with(&service, &redis, &pool, &collection, &[MARKED_POINT]).await;

    assert!(after.is_empty());
    assert!(cached_key_exists(&redis, MARKED_POINT).await?);
    restore_collection(&service).await?;
    Ok(())
}
