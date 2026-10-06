use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use sqlx::PgPool;

use crate::cache::CacheService;
use crate::modules::recommendations::live_fixture::service;

use super::semantic::{VibeSearchService, testing};

const WAITERS: usize = 16;
const WORK: Duration = Duration::from_millis(300);

async fn vibe(pg: PgPool) -> anyhow::Result<Arc<VibeSearchService>> {
    let recommendations = service(pg.clone()).await?;
    let redis = deadpool_redis::Config::from_url(
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned()),
    )
    .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(VibeSearchService::new(
        pg,
        CacheService::new(redis),
        recommendations,
    ))
}

fn key(name: &str) -> String {
    format!("stampede:{name}:{}", std::process::id())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn sixteen_identical_misses_do_the_expensive_work_once(pg: PgPool) -> anyhow::Result<()> {
    let vibe = vibe(pg).await?;
    let runs = Arc::new(AtomicUsize::new(0));
    let key = key("same");

    let answers = futures::future::join_all((0..WAITERS).map(|_| {
        let vibe = vibe.clone();
        let runs = runs.clone();
        let key = key.clone();
        async move {
            testing::expensive_once(&vibe, &key, move || {
                let runs = runs.clone();
                async move {
                    runs.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(WORK).await;
                    Ok(42_u32)
                }
            })
            .await
        }
    }))
    .await;

    for answer in &answers {
        assert_eq!(
            *answer.as_ref().expect("every waiter is answered"),
            42,
            "a waiter got something other than what the one run produced"
        );
    }
    assert_eq!(
        runs.load(Ordering::SeqCst),
        1,
        "sixteen requests for the same cold key ran the expensive path {} times; on a popular \
         query going cold this is the whole load arriving at Qdrant and PostgreSQL at once",
        runs.load(Ordering::SeqCst)
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Qdrant, Redis and NATS"]
async fn two_different_keys_are_not_made_to_wait_for_each_other(pg: PgPool) -> anyhow::Result<()> {
    let vibe = vibe(pg).await?;
    let runs = Arc::new(AtomicUsize::new(0));

    let started = std::time::Instant::now();
    let answers = futures::future::join_all((0..WAITERS).map(|index| {
        let vibe = vibe.clone();
        let runs = runs.clone();
        let key = key(&format!("different-{index}"));
        async move {
            testing::expensive_once(&vibe, &key, move || {
                let runs = runs.clone();
                async move {
                    runs.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(WORK).await;
                    Ok(index as u32)
                }
            })
            .await
        }
    }))
    .await;
    let waited = started.elapsed();

    assert_eq!(
        runs.load(Ordering::SeqCst),
        WAITERS,
        "different keys must each do their own work, or coalescing has started hiding answers"
    );
    for (index, answer) in answers.iter().enumerate() {
        assert_eq!(
            *answer.as_ref().expect("every waiter is answered"),
            index as u32
        );
    }
    assert!(
        waited < WORK * 4,
        "sixteen different keys took {waited:?}; they were serialised behind one another"
    );
    Ok(())
}

async fn catalog_search(pg: PgPool) -> anyhow::Result<Arc<super::SearchService>> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(super::SearchService::new(pg, CacheService::new(redis)))
}

#[sqlx::test(migrations = "./migrations")]
async fn sixteen_identical_catalog_misses_search_postgres_once(pg: PgPool) -> anyhow::Result<()> {
    let search = catalog_search(pg).await?;
    let runs = Arc::new(AtomicUsize::new(0));
    let key = key("catalog-same");

    let answers = futures::future::join_all((0..WAITERS).map(|_| {
        let search = search.clone();
        let runs = runs.clone();
        let key = key.clone();
        async move {
            search
                .cached(&key, move || {
                    let runs = runs.clone();
                    async move {
                        runs.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(WORK).await;
                        Ok(7_u32)
                    }
                })
                .await
        }
    }))
    .await;

    for answer in &answers {
        assert_eq!(*answer.as_ref().expect("every waiter is answered"), 7);
    }
    assert_eq!(
        runs.load(Ordering::SeqCst),
        1,
        "sixteen callers asking `/search/db/tracks` for the same phrase ran the catalog \
         search {} times; a popular phrase whose cache just expired brings all of that to \
         PostgreSQL at once",
        runs.load(Ordering::SeqCst)
    );
    Ok(())
}

const STAMPEDE: usize = 8;

#[sqlx::test(migrations = "./migrations")]
async fn a_timed_out_catalog_search_answers_every_waiter_at_once(pg: PgPool) -> anyhow::Result<()> {
    let search = catalog_search(pg).await?;
    let runs = Arc::new(AtomicUsize::new(0));
    let key = key("catalog-timeout");

    let started = std::time::Instant::now();
    let answers = futures::future::join_all((0..STAMPEDE).map(|_| {
        let search = search.clone();
        let runs = runs.clone();
        let key = key.clone();
        async move {
            search
                .cached(&key, move || {
                    let runs = runs.clone();
                    async move {
                        runs.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(WORK).await;
                        Err::<u32, _>(super::failure::search_timeout())
                    }
                })
                .await
        }
    }))
    .await;
    let waited = started.elapsed();

    for answer in &answers {
        let error = answer.as_ref().expect_err("a timed out search is an error");
        assert_eq!(error.public_code(), "search_timeout");
    }
    assert_eq!(
        runs.load(Ordering::SeqCst),
        1,
        "callers waiting on a phrase that timed out ran it again one after another"
    );
    assert!(
        waited < WORK * 4,
        "{STAMPEDE} callers of a timed out phrase took {waited:?}; the waiters queued for \
         their own rerun instead of sharing the leader's answer"
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn two_different_catalog_phrases_do_not_queue_behind_each_other(
    pg: PgPool,
) -> anyhow::Result<()> {
    let search = catalog_search(pg).await?;
    let runs = Arc::new(AtomicUsize::new(0));

    let started = std::time::Instant::now();
    futures::future::join_all((0..WAITERS).map(|index| {
        let search = search.clone();
        let runs = runs.clone();
        let key = key(&format!("catalog-different-{index}"));
        async move {
            search
                .cached(&key, move || {
                    let runs = runs.clone();
                    async move {
                        runs.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(WORK).await;
                        Ok(index as u32)
                    }
                })
                .await
        }
    }))
    .await;
    let waited = started.elapsed();

    assert_eq!(
        runs.load(Ordering::SeqCst),
        WAITERS,
        "different phrases must each do their own work"
    );
    assert!(
        waited < WORK * 4,
        "sixteen distinct phrases took {waited:?}; coalescing must key on the phrase, not \
         serialize the whole endpoint"
    );
    Ok(())
}
