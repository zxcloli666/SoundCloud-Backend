use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use sqlx::PgPool;

use super::failure;
use super::service::{SearchService, testing};
use crate::cache::CacheService;
use crate::error::AppError;

const WAITERS: usize = 16;

fn offline_service() -> anyhow::Result<Arc<SearchService>> {
    let pg = PgPool::connect_lazy("postgres://search@127.0.0.1:1/offline")?;
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    Ok(SearchService::new(pg, CacheService::new(redis), false))
}

fn assert_retryable(error: AppError, code: &str) {
    assert_eq!(error.public_code(), code);
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response
            .headers()
            .get(header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        Some("2")
    );
}

#[tokio::test]
async fn sixteen_waiters_on_a_failing_search_run_it_once() -> anyhow::Result<()> {
    let search = offline_service()?;
    let runs = Arc::new(AtomicUsize::new(0));
    let key = format!("search-failure:{}", std::process::id());

    let answers = futures::future::join_all((0..WAITERS).map(|_| {
        let search = search.clone();
        let runs = runs.clone();
        let key = key.clone();
        async move {
            testing::expensive_once(&search, &key, move || async move {
                runs.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(300)).await;
                Err(failure::from_db(sqlx::Error::PoolTimedOut))
            })
            .await
        }
    }))
    .await;

    assert_eq!(
        runs.load(Ordering::SeqCst),
        1,
        "a failing search must be shared by everyone waiting on it, not retried one waiter at a time"
    );
    for answer in answers {
        assert_retryable(
            answer.expect_err("every waiter sees the failure"),
            "search_busy",
        );
    }
    Ok(())
}

#[tokio::test]
async fn an_unexpected_failure_is_shared_and_stays_internal() -> anyhow::Result<()> {
    let search = offline_service()?;
    let key = format!("search-internal:{}", std::process::id());

    let answers = futures::future::join_all((0..2).map(|_| {
        testing::expensive_once(&search, &key, || async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            Err(AppError::internal("boom"))
        })
    }))
    .await;

    for answer in answers {
        let error = answer.expect_err("the failure reaches every waiter");
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
    Ok(())
}

#[test]
fn an_exhausted_pool_is_search_busy() {
    assert_retryable(failure::from_db(sqlx::Error::PoolTimedOut), "search_busy");
    assert!(matches!(
        failure::from_db(sqlx::Error::RowNotFound),
        AppError::Db(_)
    ));
}

#[sqlx::test(migrations = false)]
async fn a_canceled_statement_is_search_timeout(pool: PgPool) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL statement_timeout = 10")
        .execute(&mut *tx)
        .await?;
    let error = sqlx::query("SELECT pg_sleep(1)")
        .execute(&mut *tx)
        .await
        .expect_err("the statement outlives its timeout");

    assert_retryable(failure::from_db(error), "search_timeout");
    Ok(())
}
