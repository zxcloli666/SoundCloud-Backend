use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use super::*;

const BEFORE_CORE_DROP: i64 = 435;

const API_READ_INDEXES: [&str; 8] = [
    "artists_discover_az_idx",
    "artists_discover_listeners_idx",
    "artists_discover_popular_idx",
    "artists_discover_star_idx",
    "artists_discover_tracks_idx",
    "artists_discover_trending_idx",
    "artists_search_doc_gin",
    "artists_tags_gin",
];

#[sqlx::test(migrations = false)]
async fn a_split_core_drops_the_artists_indexes_only_the_api_reads(
    pool: PgPool,
) -> anyhow::Result<()> {
    core_through(BEFORE_CORE_DROP).run(&pool).await?;
    sqlx::query("DROP INDEX artists_tags_gin")
        .execute(&pool)
        .await?;

    let node = node_pool(&pool, "core", "split").await?;
    run_core(&node, None).await?;

    assert!(remaining_api_indexes(&node).await?.is_empty());
    assert!(index_exists(&node, "artists_genius_claim_idx").await?);
    assert!(index_exists(&node, "artists_normalized_name_uq").await?);
    node.close().await;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_api_node_keeps_the_artists_indexes_it_reads(pool: PgPool) -> anyhow::Result<()> {
    core_through(BEFORE_CORE_DROP).run(&pool).await?;

    let node = node_pool(&pool, "api", "split").await?;
    run_core(&node, None).await?;

    assert_eq!(remaining_api_indexes(&node).await?, API_READ_INDEXES);
    node.close().await;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_legacy_core_keeps_the_artists_indexes_its_api_reads(pool: PgPool) -> anyhow::Result<()> {
    run_core(&pool, None).await?;

    assert_eq!(remaining_api_indexes(&pool).await?, API_READ_INDEXES);
    Ok(())
}

async fn node_pool(pool: &PgPool, role: &str, mode: &str) -> anyhow::Result<PgPool> {
    let options = (*pool.connect_options()).clone();
    Ok(PgPoolOptions::new()
        .max_connections(2)
        .connect_with(options.options([("scd.role", role), ("scd.mode", mode)]))
        .await?)
}

async fn remaining_api_indexes(pool: &PgPool) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT name
         FROM unnest($1::text[]) AS api (name)
         WHERE to_regclass(name) IS NOT NULL
         ORDER BY name",
    )
    .bind(API_READ_INDEXES)
    .fetch_all(pool)
    .await?)
}

async fn index_exists(pool: &PgPool, name: &str) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(name)
        .fetch_one(pool)
        .await?)
}
