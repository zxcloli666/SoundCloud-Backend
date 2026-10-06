use std::sync::Arc;
use std::time::Duration;

use axum::response::IntoResponse;
use sc_transport::SearchType;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::SoundCloudSearch;
use super::service::{busy, chunk_prefix, unavailable};
use crate::cache::CacheService;
use crate::common::admission::{AdmissionRejection, PublicAdmission};
use crate::config::{AdmissionCfg, AdmissionLimitCfg};
use crate::error::AppError;
use crate::sc::read_tests::{SearchRelay, search_service};

fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned())
}

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

fn search(
    pool: &PgPool,
    relay: Arc<SearchRelay>,
    url: &str,
    per_session: u32,
) -> anyhow::Result<Arc<SoundCloudSearch>> {
    let redis =
        deadpool_redis::Config::from_url(url).create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(SoundCloudSearch::new(
        search_service(pool, relay)?,
        CacheService::new(redis),
        admission(url, per_session)?,
    ))
}

fn hit(id: u64) -> Value {
    json!({"id": id, "kind": "track", "title": format!("hit {id}")})
}

fn chained(total: u64) -> Arc<SearchRelay> {
    SearchRelay::answering(
        Box::new(move |inputs| {
            let offset = inputs["cursor"]
                .as_str()
                .and_then(|cursor| cursor.rsplit('=').next())
                .and_then(|offset| offset.parse::<u64>().ok())
                .unwrap_or(0);
            let last = (offset + 20).min(total);
            let next = (last < total).then(|| format!("https://next?offset={last}"));
            Some(json!({
                "ok": true,
                "collection": (offset + 1..=last).map(hit).collect::<Vec<_>>(),
                "next_href": next,
            }))
        }),
        500,
        json!({}),
    )
}

fn ids(page: &crate::cache::ListPageResult<Value>) -> Vec<u64> {
    page.collection
        .iter()
        .filter_map(|item| item["id"].as_u64())
        .collect()
}

fn query() -> String {
    format!("night drive {}", Uuid::now_v7())
}

fn code_of(error: &AppError) -> &'static str {
    error.public_code()
}

async fn quiet_body(error: AppError) -> anyhow::Result<(u16, String, Value)> {
    let response = error.into_response();
    let status = response.status().as_u16();
    let retry_after = response.headers()[axum::http::header::RETRY_AFTER]
        .to_str()?
        .to_owned();
    let body = axum::body::to_bytes(response.into_body(), 4096).await?;
    let text = String::from_utf8_lossy(&body).to_lowercase();
    for loud in ["rate limit", "rate-limited", "too many requests"] {
        assert!(!text.contains(loud), "{text}");
    }
    Ok((status, retry_after, serde_json::from_slice(&body)?))
}

#[test]
fn the_cache_key_folds_case_and_whitespace_but_not_the_type() {
    assert_eq!(
        chunk_prefix(SearchType::Tracks, "Nothing  Else\tMatters"),
        chunk_prefix(SearchType::Tracks, "nothing else matters")
    );
    assert_ne!(
        chunk_prefix(SearchType::Tracks, "x"),
        chunk_prefix(SearchType::Users, "x")
    );
    assert!(chunk_prefix(SearchType::Users, "x").starts_with("sc-search:v1:users:"));
}

#[tokio::test]
async fn failures_answer_a_coded_503_an_old_desktop_never_takes_for_a_session_problem()
-> anyhow::Result<()> {
    let (status, retry_after, body) = quiet_body(unavailable(37)).await?;
    assert_eq!((status, retry_after.as_str()), (503, "37"));
    assert_eq!(body["code"], "soundcloud_search_unavailable");

    let (status, retry_after, body) = quiet_body(busy(AdmissionRejection::Limited {
        retry_after_seconds: 12,
    }))
    .await?;
    assert_eq!((status, retry_after.as_str()), (503, "12"));
    assert_eq!(body["code"], "soundcloud_search_busy");

    let (_, retry_after, _) = quiet_body(busy(AdmissionRejection::Unavailable)).await?;
    assert_eq!(retry_after, "1");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_blank_query_or_a_page_past_the_depth_never_goes_upstream(
    pool: PgPool,
) -> anyhow::Result<()> {
    let relay = chained(1000);
    let search = search(&pool, relay.clone(), "redis://127.0.0.1:1", 10)?;

    let blank = search
        .page(Uuid::now_v7(), SearchType::Tracks, "   ", 3, 20)
        .await?;
    assert!(blank.collection.is_empty() && !blank.has_more);
    assert_eq!((blank.page, blank.page_size), (3, 20));

    let deep = search
        .page(Uuid::now_v7(), SearchType::Tracks, "x", 24, 50)
        .await?;
    assert!(deep.collection.is_empty() && !deep.has_more);
    assert_eq!(relay.lua_calls(), 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn without_redis_a_search_is_busy_rather_than_unmetered(pool: PgPool) -> anyhow::Result<()> {
    let relay = chained(40);
    let search = search(&pool, relay.clone(), "redis://127.0.0.1:1", 10)?;

    let error = search
        .page(Uuid::now_v7(), SearchType::Tracks, "x", 0, 20)
        .await
        .expect_err("admission cannot be checked");

    assert_eq!(code_of(&error), "soundcloud_search_busy");
    assert_eq!(relay.lua_calls(), 0);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn pages_slice_soundcloud_order_across_chunks_and_follow_next_href(
    pool: PgPool,
) -> anyhow::Result<()> {
    let relay = chained(45);
    let search = search(&pool, relay.clone(), &redis_url(), 100)?;
    let q = query();
    let session = Uuid::now_v7();

    let page = search.page(session, SearchType::Tracks, &q, 6, 3).await?;
    assert_eq!(ids(&page), vec![19, 20, 21]);
    assert!(page.has_more);
    assert_eq!(page.collection[0]["urn"], "soundcloud:tracks:19");

    let cursors: Vec<Value> = relay
        .lua_inputs()
        .iter()
        .map(|inputs| inputs["cursor"].clone())
        .collect();
    assert_eq!(cursors, vec![Value::Null, json!("https://next?offset=20")]);

    let second = search.page(session, SearchType::Tracks, &q, 1, 20).await?;
    assert_eq!(ids(&second), (21..=40).collect::<Vec<_>>());
    assert!(second.has_more);
    assert_eq!(
        relay.lua_calls(),
        2,
        "a repeat is served from the chunk cache"
    );

    let all = search.page(session, SearchType::Tracks, &q, 0, 50).await?;
    assert_eq!(ids(&all), (1..=45).collect::<Vec<_>>());
    assert!(!all.has_more, "SoundCloud has nothing after the last chunk");
    assert_eq!(relay.lua_calls(), 3);

    let last = search.page(session, SearchType::Tracks, &q, 2, 20).await?;
    assert_eq!(ids(&last), (41..=45).collect::<Vec<_>>());
    assert!(!last.has_more);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn the_depth_cap_ends_paging_at_three_hundred_items(pool: PgPool) -> anyhow::Result<()> {
    let relay = chained(10_000);
    let search = search(&pool, relay.clone(), &redis_url(), 100)?;
    let q = query();
    let session = Uuid::now_v7();

    let page = search.page(session, SearchType::Tracks, &q, 14, 20).await?;
    assert_eq!(ids(&page), (281..=300).collect::<Vec<_>>());
    assert!(!page.has_more);
    assert_eq!(relay.lua_calls(), 15);

    let before = search.page(session, SearchType::Tracks, &q, 13, 20).await?;
    assert!(before.has_more);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn concurrent_identical_misses_go_upstream_once(pool: PgPool) -> anyhow::Result<()> {
    let relay = chained(20);
    let search = search(&pool, relay.clone(), &redis_url(), 100)?;
    let q = query();

    let pages = futures::future::join_all((0..8).map(|_| {
        let search = search.clone();
        let q = q.clone();
        async move {
            search
                .page(Uuid::now_v7(), SearchType::Tracks, &q, 0, 20)
                .await
        }
    }))
    .await;

    for page in pages {
        assert_eq!(ids(&page?).len(), 20);
    }
    assert_eq!(relay.lua_calls(), 1);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_failed_chunk_is_answered_from_its_marker_without_going_upstream(
    pool: PgPool,
) -> anyhow::Result<()> {
    for status in [401, 403, 429, 500] {
        let relay = SearchRelay::answering(Box::new(|_| None), status, json!({"error": "x"}));
        let search = search(&pool, relay.clone(), &redis_url(), 100)?;
        let q = query();

        let error = search
            .page(Uuid::now_v7(), SearchType::Tracks, &q, 0, 20)
            .await
            .expect_err("SoundCloud did not answer");
        assert_eq!(code_of(&error), "soundcloud_search_unavailable", "{status}");
        let (status_code, retry_after, _) = quiet_body(error).await?;
        assert_eq!(status_code, 503);
        assert!(retry_after.parse::<u64>()? >= 5);

        let (lua, proxy) = (relay.lua_calls(), relay.proxy_searches());
        let again = search
            .page(Uuid::now_v7(), SearchType::Tracks, &q, 0, 20)
            .await
            .expect_err("the marker answers");
        assert_eq!(code_of(&again), "soundcloud_search_unavailable");
        assert_eq!((relay.lua_calls(), relay.proxy_searches()), (lua, proxy));
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_query_soundcloud_rejects_is_an_empty_page(pool: PgPool) -> anyhow::Result<()> {
    for status in [400, 404, 422] {
        let relay = SearchRelay::answering(Box::new(|_| None), status, json!({}));
        let search = search(&pool, relay, &redis_url(), 100)?;

        let page = search
            .page(Uuid::now_v7(), SearchType::Users, &query(), 0, 20)
            .await?;
        assert!(page.collection.is_empty() && !page.has_more, "{status}");
    }
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn a_session_past_its_budget_is_told_search_is_busy(pool: PgPool) -> anyhow::Result<()> {
    let relay = chained(20);
    let search = search(&pool, relay.clone(), &redis_url(), 1)?;
    let session = Uuid::now_v7();

    search
        .page(session, SearchType::Tracks, &query(), 0, 20)
        .await?;
    let error = search
        .page(session, SearchType::Tracks, &query(), 0, 20)
        .await
        .expect_err("the budget is spent");

    assert_eq!(code_of(&error), "soundcloud_search_busy");
    assert_eq!(relay.lua_calls(), 1);
    Ok(())
}
