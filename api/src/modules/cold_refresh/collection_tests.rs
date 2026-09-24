use backend_contracts::CatalogCollection;
use sqlx::PgPool;
use uuid::Uuid;

use super::collection::ensure;

async fn install(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!("../../../migrations/0057_background_jobs.sql"))
        .execute(pool)
        .await?;
    sqlx::raw_sql(include_str!(
        "../../../migrations/0090_catalog_collection_progress.sql"
    ))
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_cold_read_enqueues_without_a_session_token_or_redis_and_coalesces_by_scope(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    let (first, second) = tokio::join!(
        ensure(&pool, CatalogCollection::LikedTracks, "42", true, 600),
        ensure(
            &pool,
            CatalogCollection::LikedTracks,
            "soundcloud:users:42",
            true,
            600
        )
    );
    for result in [first?, second?] {
        assert_eq!(result.status, "refreshing");
        assert!(result.last_completed_at.is_none());
        assert_eq!(result.retry_after_seconds, 5);
    }
    ensure(&pool, CatalogCollection::LikedTracks, "42", false, 600).await?;
    let keys: Vec<(String, i64)> =
        sqlx::query_as("SELECT dedup_key, generation FROM background_jobs ORDER BY dedup_key")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        keys,
        [
            ("liked-tracks:42:owner".into(), 1),
            ("liked-tracks:42:public".into(), 1)
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn only_a_completed_snapshot_can_make_a_collection_fresh(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    sqlx::query("INSERT INTO catalog_collection_sync (subject_id, collection, scope, job_id, generation, snapshot_id, page_count)
        VALUES ('42', 'followings', 'owner', $1, 1, $2, 12)")
        .bind(Uuid::now_v7()).bind(Uuid::now_v7()).execute(&pool).await?;
    assert_eq!(
        ensure(&pool, CatalogCollection::Followings, "42", true, 600)
            .await?
            .status,
        "refreshing"
    );
    sqlx::query("DELETE FROM background_jobs")
        .execute(&pool)
        .await?;
    sqlx::query("UPDATE catalog_collection_sync SET complete = true, synced_at = now()")
        .execute(&pool)
        .await?;
    let ready = ensure(&pool, CatalogCollection::Followings, "42", true, 600).await?;
    assert_eq!(ready.status, "ready");
    assert!(ready.last_completed_at.is_some());
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(queued, 0);
    assert_eq!(
        ensure(&pool, CatalogCollection::Followings, "42", false, 600)
            .await?
            .status,
        "refreshing"
    );
    sqlx::query("UPDATE catalog_collection_sync SET synced_at = now() - interval '1 hour'")
        .execute(&pool)
        .await?;
    assert_eq!(
        ensure(&pool, CatalogCollection::Followings, "42", true, 600)
            .await?
            .status,
        "refreshing"
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_invalid_collection_identity_cannot_schedule_work(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    assert!(
        ensure(
            &pool,
            CatalogCollection::OwnedTracks,
            "42/tracks",
            true,
            600
        )
        .await
        .is_err()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_background_cooldown_survives_reads_and_delays_frontend_polling(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    ensure(&pool, CatalogCollection::LikedTracks, "42", true, 600).await?;
    sqlx::query("UPDATE background_jobs SET available_at = now() + interval '15 minutes'")
        .execute(&pool)
        .await?;
    let sync = ensure(&pool, CatalogCollection::LikedTracks, "42", true, 600).await?;
    assert!((899..=900).contains(&sync.retry_after_seconds));
    let generation: i64 = sqlx::query_scalar("SELECT generation FROM background_jobs")
        .fetch_one(&pool)
        .await?;
    assert_eq!(generation, 1);
    Ok(())
}
