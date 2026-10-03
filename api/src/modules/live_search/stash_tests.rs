use std::sync::Arc;

use serde_json::{Value, json};
use sqlx::PgPool;

use super::slim;
use super::stash::{Adoption, LiveStash};
use super::store::{LiveStore, Window};
use crate::cache::CacheService;
use crate::modules::indexing::IndexingService;
use crate::modules::tracks::TrackPriority;

fn cache() -> anyhow::Result<Arc<CacheService>> {
    let redis = deadpool_redis::Config::from_url(
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned()),
    )
    .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(CacheService::new(redis))
}

async fn indexing(pool: &PgPool) -> anyhow::Result<Arc<IndexingService>> {
    let nats = crate::bus::nats::NatsService::connect(
        "nats://127.0.0.1:1",
        tokio_util::sync::CancellationToken::new(),
    )
    .await?;
    let background = crate::background_jobs::BackgroundJobs::new(nats);
    Ok(IndexingService::new(
        pool.clone(),
        background.clone(),
        crate::background_jobs::IndexingJobs::new(background),
        420_000,
    ))
}

fn unique_id() -> u64 {
    8_000_000_000 + u64::from(uuid::Uuid::now_v7().as_fields().1) * 1000
}

fn sighted_track(id: u64) -> Value {
    let mut raw = json!({
        "id": id,
        "kind": "track",
        "title": "Seen in search",
        "duration": 200000,
        "full_duration": 200000,
        "sharing": "public",
        "policy": "ALLOW",
        "publisher_metadata": {"artist": "Label Artist"},
        "user": {"id": id + 1, "kind": "user", "username": "Uploader", "followers_count": 12}
    });
    sc_transport::normalize_v2_to_v1(&mut raw);
    raw
}

async fn sight(cache: &Arc<CacheService>, raw: &[Value], kind: super::query::LiveKind) {
    let slimmed = slim::slim(kind, raw);
    let ids = slimmed
        .items
        .iter()
        .filter_map(|item| item["urn"].as_str().map(str::to_owned))
        .collect();
    LiveStore::new(cache.clone())
        .write(
            "tracks",
            &uuid::Uuid::now_v7().simple().to_string(),
            &Window::new(ids, chrono::Utc::now().timestamp()),
            60,
            &slimmed.items,
            &slimmed.users,
        )
        .await;
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_sighted_track_is_adopted_and_a_real_observation_overwrites_it(
    pool: PgPool,
) -> anyhow::Result<()> {
    let cache = cache()?;
    let id = unique_id();
    let urn = format!("soundcloud:tracks:{id}");
    sight(&cache, &[sighted_track(id)], super::query::LiveKind::Tracks).await;
    let indexing = indexing(&pool).await?;
    let stash = LiveStash::new(cache);

    assert!(
        stash.adopt_track(&indexing, &urn).await.was_seen(),
        "without NATS the pipeline kick may outlast the cap, the row is written before it"
    );
    let (title, artist, sharing): (String, Option<String>, String) =
        sqlx::query_as("SELECT title, metadata_artist, sharing FROM tracks WHERE sc_track_id = $1")
            .bind(id.to_string())
            .fetch_one(&pool)
            .await?;
    assert_eq!(title, "Seen in search");
    assert_eq!(artist.as_deref(), Some("Label Artist"));
    assert_eq!(sharing, "public");

    let mut refreshed = sighted_track(id);
    refreshed["title"] = json!("Renamed upstream");
    refreshed["last_modified"] = json!("2026-10-01T00:00:00Z");
    indexing
        .ingest_track_from_sc(
            &refreshed,
            TrackPriority::Discovery,
            catalog_ingest::Observation::begin(&pool).await?,
        )
        .await?;
    let title: String = sqlx::query_scalar("SELECT title FROM tracks WHERE sc_track_id = $1")
        .bind(id.to_string())
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        title, "Renamed upstream",
        "an unverified adoption must never outrank a real observation"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn an_unsighted_entity_is_left_to_the_refresh_path(pool: PgPool) -> anyhow::Result<()> {
    let stash = LiveStash::new(cache()?);
    let id = unique_id();

    assert_eq!(
        stash
            .adopt_track(&indexing(&pool).await?, &format!("soundcloud:tracks:{id}"))
            .await,
        Adoption::Unseen
    );
    assert_eq!(
        stash
            .adopt_user(&pool, &format!("soundcloud:users:{id}"))
            .await,
        Adoption::Unseen
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM tracks")
        .fetch_one(&pool)
        .await?;
    assert_eq!(rows, 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn an_uploader_and_a_playlist_seen_in_search_are_adopted(pool: PgPool) -> anyhow::Result<()> {
    let cache = cache()?;
    let id = unique_id();
    sight(&cache, &[sighted_track(id)], super::query::LiveKind::Tracks).await;
    let mut raw_playlist = json!({
        "id": id, "kind": "playlist", "title": "Seen mix", "track_count": 4, "sharing": "public",
        "user": {"id": id + 1, "kind": "user", "username": "Uploader"}
    });
    sc_transport::normalize_v2_to_v1(&mut raw_playlist);
    sight(&cache, &[raw_playlist], super::query::LiveKind::Playlists).await;
    let stash = LiveStash::new(cache);

    assert_eq!(
        stash
            .adopt_user(&pool, &format!("soundcloud:users:{}", id + 1))
            .await,
        Adoption::Adopted
    );
    assert_eq!(
        stash
            .adopt_playlist(&pool, &format!("soundcloud:playlists:{id}"))
            .await,
        Adoption::Adopted
    );
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE sc_user_id = $1")
        .bind((id + 1).to_string())
        .fetch_one(&pool)
        .await?;
    let title: String = sqlx::query_scalar("SELECT title FROM playlists WHERE sc_playlist_id = $1")
        .bind(id.to_string())
        .fetch_one(&pool)
        .await?;
    assert_eq!(
        (username.as_str(), title.as_str()),
        ("Uploader", "Seen mix")
    );
    Ok(())
}
