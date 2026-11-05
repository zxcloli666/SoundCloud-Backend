use std::sync::Arc;

use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::PublicUsers;
use crate::modules::resolve::miss_tests::{miss_with, redis_url, relay};
use crate::modules::users::{UserRepository, UserRow};
use crate::sc::read_tests::SearchRelay;

const OFFLINE_REDIS: &str = "redis://127.0.0.1:1";

async fn seed(pool: &PgPool, synced_ago: &str) -> anyhow::Result<UserRow> {
    sqlx::query(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized, avatar_url, sc_synced_at)
         VALUES ('17', 'soundcloud:users:17', 'Owner', 'owner', 'https://i1.sndcdn.com/old.jpg', now() - $1::interval)",
    )
    .bind(synced_ago)
    .execute(pool)
    .await?;
    load(pool).await
}

async fn load(pool: &PgPool) -> anyhow::Result<UserRow> {
    UserRepository::new(pool.clone())
        .find_by_urn("17")
        .await?
        .ok_or_else(|| anyhow::anyhow!("user 17 is missing"))
}

async fn jobs(pool: &PgPool) -> anyhow::Result<Vec<(String, i16)>> {
    Ok(
        sqlx::query_as("SELECT dedup_key, priority FROM background_jobs ORDER BY dedup_key")
            .fetch_all(pool)
            .await?,
    )
}

fn new_avatar() -> Arc<SearchRelay> {
    SearchRelay::answering(
        Box::new(|_| {
            Some(json!({
                "ok": true,
                "user": {"id": 17, "urn": "soundcloud:users:17", "kind": "user", "username": "Owner",
                         "permalink": "owner", "avatar_url": "https://i1.sndcdn.com/new.jpg",
                         "last_modified": "2026-11-04T16:02:47Z",
                         "visuals": {"urn": "soundcloud:users:17", "enabled": true, "tracking": null,
                                     "visuals": [{"urn": "soundcloud:visuals:278279490", "entry_time": 0,
                                                  "visual_url": "https://i1.sndcdn.com/visuals-000000000017-LpaCgv-original.jpg"}]}},
            }))
        }),
        500,
        Value::Null,
    )
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_fresh_user_is_served_without_soundcloud(pool: PgPool) -> anyhow::Result<()> {
    let row = seed(&pool, "1 minute").await?;
    let relay = relay();
    let public = PublicUsers::new(
        pool.clone(),
        miss_with(&pool, relay.clone(), OFFLINE_REDIS, 100).await?,
    );

    assert!(!public.refresh(Uuid::now_v7(), "42", &row).await);
    assert_eq!(relay.lua_calls(), 0);
    assert!(jobs(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn an_unanswered_read_leaves_one_viewer_refresh_and_is_not_repeated(
    pool: PgPool,
) -> anyhow::Result<()> {
    let row = seed(&pool, "1 hour").await?;
    let public = PublicUsers::new(
        pool.clone(),
        miss_with(&pool, relay(), OFFLINE_REDIS, 100).await?,
    );

    assert!(!public.refresh(Uuid::now_v7(), "42", &row).await);
    assert_eq!(jobs(&pool).await?, vec![("user:17:public".to_owned(), 15)]);

    sqlx::query("DELETE FROM background_jobs")
        .execute(&pool)
        .await?;
    assert!(!public.refresh(Uuid::now_v7(), "42", &row).await);
    assert!(jobs(&pool).await?.is_empty());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
#[ignore = "requires a local Redis"]
async fn a_stale_user_is_read_through_apiv2_at_once(pool: PgPool) -> anyhow::Result<()> {
    let row = seed(&pool, "1 hour").await?;
    let relay = new_avatar();
    let public = PublicUsers::new(
        pool.clone(),
        miss_with(&pool, relay.clone(), &redis_url(), 100).await?,
    );

    assert!(public.refresh(Uuid::now_v7(), "42", &row).await);
    assert_eq!(relay.lua_calls(), 1);
    let stored = load(&pool).await?;
    assert_eq!(
        stored.avatar_url.as_deref(),
        Some("https://i1.sndcdn.com/new.jpg")
    );
    assert_eq!(
        stored.banner_url.as_deref(),
        Some("https://i1.sndcdn.com/visuals-000000000017-LpaCgv-original.jpg")
    );
    assert!(jobs(&pool).await?.is_empty());
    Ok(())
}
