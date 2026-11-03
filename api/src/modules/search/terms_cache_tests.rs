use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use super::terms::{Shape, TermRow, TermsCache, build, resolve};

const WAITERS: usize = 16;
const QUERY: &str = "umbrela rihana";

async fn wide(small: &PgPool) -> anyhow::Result<PgPool> {
    Ok(PgPoolOptions::new()
        .max_connections(40)
        .connect_with((*small.connect_options()).clone())
        .await?)
}

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO search_terms (word, ndoc, latin)
         SELECT word, ndoc, search_latin(word)
         FROM (VALUES ('umbrella', 90), ('rihanna', 80), ('rihana', 3), ('blinding', 40), ('lights', 50)) AS v (word, ndoc)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn waiting_on_lexicon(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity
         WHERE datname = current_database()
           AND wait_event_type = 'Lock'
           AND query LIKE '%FROM search_terms%'",
    )
    .fetch_one(pool)
    .await?)
}

async fn uncached(pool: &PgPool, q: &str, shape: Shape) -> anyhow::Result<Vec<TermRow>> {
    let cap = match shape {
        Shape::Lyrics => 8,
        Shape::Tracks | Shape::Entities => 6,
    };
    Ok(
        sqlx::query_file_as!(TermRow, "queries/search/terms.sql", q, cap)
            .fetch_all(pool)
            .await?,
    )
}

#[sqlx::test(migrations = "./migrations")]
async fn concurrent_identical_searches_make_one_lexicon_call(small: PgPool) -> anyhow::Result<()> {
    let pool = wide(&small).await?;
    seed(&pool).await?;
    let cache = Arc::new(TermsCache::new());

    let mut blocker = pool.begin().await?;
    sqlx::query("LOCK TABLE search_terms IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await?;
    let calls: Vec<_> = (0..WAITERS)
        .map(|_| {
            let pool = pool.clone();
            let cache = cache.clone();
            tokio::spawn(async move {
                let mut tx = pool.begin().await?;
                let terms = resolve(&cache, &mut tx, QUERY, Shape::Tracks).await?;
                anyhow::Ok(terms)
            })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(700)).await;
    let waiting = waiting_on_lexicon(&pool).await?;
    blocker.rollback().await?;

    let mut answers = Vec::new();
    for call in calls {
        answers.push(call.await??);
    }
    assert_eq!(waiting, 1, "one statement waits, the others share it");
    assert!(answers.windows(2).all(|pair| pair[0] == pair[1]));
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn without_the_cache_every_search_calls_the_lexicon(small: PgPool) -> anyhow::Result<()> {
    let pool = wide(&small).await?;
    seed(&pool).await?;

    let mut blocker = pool.begin().await?;
    sqlx::query("LOCK TABLE search_terms IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await?;
    let calls: Vec<_> = (0..WAITERS)
        .map(|_| {
            let pool = pool.clone();
            tokio::spawn(async move { uncached(&pool, QUERY, Shape::Tracks).await })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(700)).await;
    let waiting = waiting_on_lexicon(&pool).await?;
    blocker.rollback().await?;
    for call in calls {
        call.await??;
    }
    assert_eq!(waiting, WAITERS as i64);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn cached_terms_equal_uncached_terms_for_every_shape(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool).await?;
    let cache = TermsCache::new();
    for q in [
        QUERY,
        "blinding lights",
        "umb",
        "rihanna umbrella blinding lights extra words more tokens here",
        "",
        "!!!",
        "ночь",
    ] {
        for shape in [Shape::Tracks, Shape::Entities, Shape::Lyrics] {
            let expected = build(&uncached(&pool, q, shape).await?, shape);
            for _ in 0..2 {
                let mut tx = pool.begin().await?;
                let got = resolve(&cache, &mut tx, q, shape).await?;
                assert_eq!(got, expected, "{q:?} {}", matches!(shape, Shape::Lyrics));
            }
        }
    }
    Ok(())
}
