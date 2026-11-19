use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::http::StatusCode;
use axum::routing::put;
use serde_json::Value;
use sqlx::PgPool;

use super::SyncQueueHandler;
use super::unclear_create_tests::{handler, run};

const USER: &str = "17";

async fn serve(status: StatusCode) -> anyhow::Result<(std::net::SocketAddr, Arc<AtomicUsize>)> {
    let hits = Arc::new(AtomicUsize::new(0));
    let counted = hits.clone();
    let answer = move || {
        let counted = counted.clone();
        async move {
            counted.fetch_add(1, Ordering::SeqCst);
            status
        }
    };
    let app = Router::new().route("/me/followings/{id}", put(answer.clone()).delete(answer));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok((address, hits))
}

async fn setup(
    pool: &PgPool,
    status: StatusCode,
    action: &str,
    target: &str,
) -> anyhow::Result<(SyncQueueHandler, Arc<AtomicUsize>)> {
    crate::db::migrations::run_core(pool, None).await?;
    super::track_tests::connect_owner(pool).await?;
    let (address, hits) = serve(status).await?;
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL scd.writer = 'api'")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO user_followings (user_id, target_user_urn, wanted_state) VALUES ($1, $2, $3)",
    )
    .bind(USER)
    .bind(target)
    .bind(action == "follow_user")
    .execute(&mut *tx)
    .await?;
    sqlx::query(include_str!(
        "../../../../api/queries/sync_queue/service/enqueue.sql"
    ))
    .bind(USER)
    .bind(action)
    .bind(target)
    .bind(None::<Value>)
    .bind(false)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((handler(pool, address)?, hits))
}

async fn mirror(pool: &PgPool) -> anyhow::Result<(bool, bool, bool)> {
    Ok(sqlx::query_as(
        "SELECT wanted_state, progress, synced_for_v = wanted_v FROM user_followings",
    )
    .fetch_one(pool)
    .await?)
}

async fn queued(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM sync_queue")
        .fetch_one(pool)
        .await?)
}

async fn heal(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::query("UPDATE user_followings SET synced_at = now() - interval '1 hour'")
        .execute(pool)
        .await?;
    crate::handlers::maintenance::MaintenanceHandler::new(pool.clone())
        .heal_sync_queue()
        .await
        .map_err(|error| anyhow::anyhow!("{error:?}"))
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_queued_self_follow_closes_without_calling_soundcloud(
    pool: PgPool,
) -> anyhow::Result<()> {
    let (handler, hits) = setup(
        &pool,
        StatusCode::NO_CONTENT,
        "follow_user",
        "soundcloud:users:17",
    )
    .await?;

    run(&pool, &handler).await?;

    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert_eq!(queued(&pool).await?, 0);
    assert_eq!(mirror(&pool).await?, (false, false, true));
    heal(&pool).await?;
    assert_eq!(queued(&pool).await?, 0);
    Ok(())
}

async fn refused_follow(pool: &PgPool, status: StatusCode) -> anyhow::Result<()> {
    let (handler, hits) = setup(pool, status, "follow_user", "soundcloud:users:99").await?;

    run(pool, &handler).await?;

    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(queued(pool).await?, 0);
    assert_eq!(mirror(pool).await?, (false, false, true));
    heal(pool).await?;
    assert_eq!(queued(pool).await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_follow_refused_with_400_is_closed_and_heal_does_not_bring_it_back(
    pool: PgPool,
) -> anyhow::Result<()> {
    refused_follow(&pool, StatusCode::BAD_REQUEST).await
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_follow_refused_with_403_is_closed_and_heal_does_not_bring_it_back(
    pool: PgPool,
) -> anyhow::Result<()> {
    refused_follow(&pool, StatusCode::FORBIDDEN).await
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn an_unfollow_refused_with_403_is_closed_and_settled(pool: PgPool) -> anyhow::Result<()> {
    let (handler, hits) = setup(
        &pool,
        StatusCode::FORBIDDEN,
        "unfollow_user",
        "soundcloud:users:99",
    )
    .await?;

    run(&pool, &handler).await?;

    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(queued(&pool).await?, 0);
    assert_eq!(mirror(&pool).await?, (false, false, true));
    heal(&pool).await?;
    assert_eq!(queued(&pool).await?, 0);
    Ok(())
}

#[sqlx::test(migrator = "crate::db::migrations::TEST_CORE")]
async fn a_follow_limited_with_429_still_retries(pool: PgPool) -> anyhow::Result<()> {
    let (handler, hits) = setup(
        &pool,
        StatusCode::TOO_MANY_REQUESTS,
        "follow_user",
        "soundcloud:users:99",
    )
    .await?;

    run(&pool, &handler).await?;

    assert_eq!(hits.load(Ordering::SeqCst), 1);
    let row: (bool, i32, bool) =
        sqlx::query_as("SELECT dead, retry_count, next_run_at > now() FROM sync_queue")
            .fetch_one(&pool)
            .await?;
    assert_eq!(row, (false, 0, true));
    assert_eq!(mirror(&pool).await?, (true, false, false));
    Ok(())
}
