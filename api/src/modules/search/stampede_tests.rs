use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use sqlx::PgPool;

use crate::cache::CacheService;
use crate::modules::lyrics::WorkerClient;
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
    let cache = CacheService::new(redis);
    let qdrant = recommendations.qdrant.clone();
    let worker = WorkerClient::new(recommendations.nats.clone(), cache.clone(), qdrant.clone());
    Ok(VibeSearchService::new(
        pg,
        cache,
        recommendations,
        worker,
        qdrant,
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

async fn catalog_search(pg: PgPool) -> anyhow::Result<Arc<super::service::SearchService>> {
    let redis = deadpool_redis::Config::from_url(
        std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned()),
    )
    .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(super::service::SearchService::new(
        pg,
        CacheService::new(redis),
    ))
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
async fn sixteen_identical_catalog_misses_search_postgres_once(pg: PgPool) -> anyhow::Result<()> {
    let search = catalog_search(pg).await?;
    let runs = Arc::new(AtomicUsize::new(0));
    let key = key("catalog-same");

    let answers = futures::future::join_all((0..WAITERS).map(|_| {
        let search = search.clone();
        let runs = runs.clone();
        let key = key.clone();
        async move {
            super::service::testing::expensive_once(&search, &key, move || {
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
        "sixteen callers asking `/search/db/artists` for the same phrase ran the full-text \
         search {} times; a popular phrase whose cache just expired brings all of that to \
         PostgreSQL at once",
        runs.load(Ordering::SeqCst)
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a local Redis"]
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
            super::service::testing::expensive_once(&search, &key, move || {
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
