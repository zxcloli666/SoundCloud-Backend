use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use sqlx::PgPool;

use super::SearchService;
use super::failure::{self, search_busy, search_timeout, vibe_unavailable};
use crate::cache::CacheService;
use crate::error::AppError;

pub(crate) fn assert_quiet_body(body: &str) {
    let lower = body.to_lowercase();
    for phrase in ["rate limit", "rate-limited", "too many requests"] {
        assert!(!lower.contains(phrase), "{body}");
    }
}

async fn assert_coded(error: AppError, code: &str, retry_after: &str) -> anyhow::Result<()> {
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["retry-after"], retry_after);
    let body = axum::body::to_bytes(response.into_body(), 1 << 16).await?;
    let body = String::from_utf8(body.to_vec())?;
    let json: serde_json::Value = serde_json::from_str(&body)?;
    assert_eq!(json["code"], code);
    assert_eq!(json["statusCode"], 503);
    assert_quiet_body(&body);
    Ok(())
}

#[tokio::test]
async fn every_search_failure_is_a_quiet_coded_503() -> anyhow::Result<()> {
    assert_coded(search_timeout(), "search_timeout", "2").await?;
    assert_coded(search_busy(), "search_busy", "2").await?;
    assert_coded(vibe_unavailable(), "vibe_unavailable", "10").await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_statement_timeout_becomes_search_timeout(pool: PgPool) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL statement_timeout = 1")
        .execute(&mut *tx)
        .await?;
    let error: AppError = sqlx::query("SELECT pg_sleep(1)")
        .execute(&mut *tx)
        .await
        .expect_err("the statement must time out")
        .into();
    assert_coded(failure::map(error), "search_timeout", "2").await
}

#[test]
fn a_pool_wait_is_busy_and_other_errors_pass_through() {
    assert!(matches!(
        failure::map(AppError::Db(sqlx::Error::PoolTimedOut)),
        AppError::Coded {
            code: "search_busy",
            ..
        }
    ));
    assert!(matches!(
        failure::map(AppError::not_found("x")),
        AppError::NotFound(_)
    ));
}

#[sqlx::test(migrations = "./migrations")]
async fn no_free_permit_answers_search_busy(pool: PgPool) -> anyhow::Result<()> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let search: Arc<SearchService> = SearchService::new(pool, CacheService::new(redis));
    let _held = search.permits.acquire_many(8).await?;
    let error = search
        .tracks("anything", None, 0, 20)
        .await
        .expect_err("every permit is held");
    assert_coded(error, "search_busy", "2").await
}
