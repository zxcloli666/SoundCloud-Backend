use std::sync::Arc;
use std::time::Duration;

use catalog_ingest::TrackPriority;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::miss::{Adopted, CatalogMiss, outcome_of};
use crate::common::admission::PublicAdmission;
use crate::config::{AdmissionCfg, AdmissionLimitCfg};
use crate::error::AppError;
use crate::sc::read_tests::{SearchRelay, search_service};

fn admission(url: &str, per_session: u32) -> anyhow::Result<Arc<PublicAdmission>> {
    let limit = AdmissionLimitCfg {
        per_client: per_session,
        global: 1_000_000,
    };
    let pool =
        deadpool_redis::Config::from_url(url).create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(PublicAdmission::new(
        pool,
        AdmissionCfg {
            window: Duration::from_secs(60),
            timeout: Duration::from_millis(500),
            max_in_flight: 64,
            login: limit,
            link_create: limit,
            resolve: limit,
            sc_search: limit,
            catalog_miss: limit,
        },
    ))
}

fn entities(id: &str) -> Value {
    let user = json!({"id": 17, "urn": "soundcloud:users:17", "kind": "user", "username": "Owner", "permalink": "owner"});
    json!({
        "ok": true,
        "track": {"id": id.parse::<u64>().unwrap_or(0), "urn": format!("soundcloud:tracks:{id}"), "kind": "track",
                  "title": format!("Remote {id}"), "duration": 120000, "sharing": "public", "user": user.clone()},
        "playlist": {"id": id.parse::<u64>().unwrap_or(0), "urn": format!("soundcloud:playlists:{id}"), "kind": "playlist",
                     "title": format!("Remote mix {id}"), "sharing": "public", "user": user.clone()},
        "user": user,
    })
}

pub(crate) fn relay() -> Arc<SearchRelay> {
    SearchRelay::answering(
        Box::new(|inputs| {
            let id = inputs["id"].as_str().unwrap_or_default().to_owned();
            Some(entities(&id))
        }),
        500,
        json!({}),
    )
}

pub(crate) async fn miss_with(
    pool: &PgPool,
    relay: Arc<SearchRelay>,
    redis_url: &str,
    per_session: u32,
) -> anyhow::Result<Arc<CatalogMiss>> {
    let nats = crate::bus::nats::NatsService::connect(
        "nats://127.0.0.1:1",
        tokio_util::sync::CancellationToken::new(),
    )
    .await?;
    let background = crate::background_jobs::BackgroundJobs::new(nats);
    let indexing = crate::modules::indexing::IndexingService::new(
        pool.clone(),
        background.clone(),
        crate::background_jobs::IndexingJobs::new(background),
        420000,
    );
    Ok(CatalogMiss::new(
        pool.clone(),
        search_service(pool, relay)?,
        indexing,
        admission(redis_url, per_session)?,
    ))
}

pub(crate) async fn offline(pool: &PgPool) -> anyhow::Result<Arc<CatalogMiss>> {
    miss_with(pool, relay(), "redis://127.0.0.1:1", 100).await
}

pub(crate) fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned())
}

async fn count(pool: &PgPool, sql: &str) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(sql).fetch_one(pool).await?)
}

#[test]
fn only_a_soundcloud_404_means_gone() {
    let status = |status| AppError::ScApi {
        status,
        body: Value::Null,
        retry_after_sec: None,
    };
    assert_eq!(outcome_of(&status(404)), Adopted::Gone);
    for other in [401, 403, 429, 500] {
        assert_eq!(outcome_of(&status(other)), Adopted::Unavailable);
    }
    assert_eq!(
        outcome_of(&AppError::ScUnreachable("down".into())),
        Adopted::Unavailable
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn no_admission_means_no_soundcloud_call(pool: PgPool) -> anyhow::Result<()> {
    let relay = relay();
    let miss = miss_with(&pool, relay.clone(), "redis://127.0.0.1:1", 100).await?;
    assert_eq!(
        miss.track(Uuid::now_v7(), "42", TrackPriority::Discovery)
            .await,
        Adopted::Unavailable
    );
    assert_eq!(miss.user(Uuid::now_v7(), "abc").await, Adopted::Unavailable);
    assert_eq!(relay.lua_calls(), 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn an_opened_entity_is_fetched_and_stored(pool: PgPool) -> anyhow::Result<()> {
    let miss = miss_with(&pool, relay(), &redis_url(), 100).await?;
    let session = Uuid::now_v7();
    assert_eq!(
        miss.track(session, "42", TrackPriority::Discovery).await,
        Adopted::Stored
    );
    assert_eq!(miss.user(session, "17").await, Adopted::Stored);
    assert_eq!(miss.playlist(session, "77").await, Adopted::Stored);
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM tracks WHERE sc_track_id = '42'"
        )
        .await?,
        1
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM users WHERE sc_user_id = '17'").await?,
        1
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM playlists WHERE sc_playlist_id = '77'"
        )
        .await?,
        1
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_playlist_write_adopts_at_most_twenty_five_unknown_tracks(
    pool: PgPool,
) -> anyhow::Result<()> {
    let miss = miss_with(&pool, relay(), &redis_url(), 1000).await?;
    let session = Uuid::now_v7();
    let three: Vec<String> = (101..=103).map(|id| id.to_string()).collect();
    miss.tracks(session, &three).await?;
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM tracks WHERE sc_track_id IN ('101', '102', '103')"
        )
        .await?,
        3
    );
    let many: Vec<String> = (201..=240).map(|id| id.to_string()).collect();
    miss.tracks(session, &many).await?;
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM tracks WHERE sc_track_id::bigint BETWEEN 201 AND 240"
        )
        .await?,
        25
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn an_exhausted_session_stops_adopting(pool: PgPool) -> anyhow::Result<()> {
    let relay = relay();
    let miss = miss_with(&pool, relay.clone(), &redis_url(), 1).await?;
    let session = Uuid::now_v7();
    assert_eq!(
        miss.track(session, "42", TrackPriority::Discovery).await,
        Adopted::Stored
    );
    assert_eq!(
        miss.track(session, "43", TrackPriority::Discovery).await,
        Adopted::Unavailable
    );
    assert_eq!(relay.lua_calls(), 1);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_stalled_soundcloud_cannot_hold_a_playlist_write(pool: PgPool) -> anyhow::Result<()> {
    let miss = miss_with(&pool, SearchRelay::stalled(), &redis_url(), 1000).await?;
    let unknown: Vec<String> = (301..=325).map(|id| id.to_string()).collect();
    let started = std::time::Instant::now();
    miss.tracks(Uuid::now_v7(), &unknown).await?;
    assert!(
        started.elapsed() < Duration::from_secs(12),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM tracks WHERE sc_track_id::bigint BETWEEN 301 AND 325"
        )
        .await?,
        0
    );
    Ok(())
}
