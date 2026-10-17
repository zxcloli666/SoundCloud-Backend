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
    sqlx::raw_sql(
        "CREATE TABLE subscriptions (user_urn text PRIMARY KEY, exp_date bigint NOT NULL);
         CREATE TABLE soundcloud_connections (
             soundcloud_user_id text NOT NULL,
             created_at timestamptz NOT NULL DEFAULT now()
         );",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn fail_for_good(
    pool: &PgPool,
    dedup_key: &str,
    last_error: &str,
    ago: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO background_job_failures (
             id, kind, lane, dedup_key, payload, priority, generation, attempts, max_attempts,
             last_error, created_at, failed_at
         ) VALUES ($1, 'catalog.collection', 'core_bulk', $2, '{}', 15, 1, 1, 8,
                   $3, now() - $4::interval, now() - $4::interval)",
    )
    .bind(Uuid::now_v7())
    .bind(dedup_key)
    .bind(last_error)
    .bind(ago)
    .execute(pool)
    .await?;
    Ok(())
}

async fn queued(pool: &PgPool) -> anyhow::Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM background_jobs")
        .fetch_one(pool)
        .await?)
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

#[sqlx::test(migrations = false)]
async fn a_star_owner_collection_is_read_ahead_of_other_owners(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    sqlx::query(
        "INSERT INTO subscriptions (user_urn, exp_date) VALUES
             ('42', extract(epoch FROM now())::bigint + 3600),
             ('43', extract(epoch FROM now())::bigint - 3600)",
    )
    .execute(&pool)
    .await?;

    for owner in ["42", "43", "44"] {
        ensure(&pool, CatalogCollection::LikedTracks, owner, true, 600).await?;
    }
    ensure(&pool, CatalogCollection::Followings, "42", false, 600).await?;
    ensure(&pool, CatalogCollection::TrackFavoriters, "42", false, 600).await?;

    let priorities: Vec<(String, i16)> =
        sqlx::query_as("SELECT dedup_key, priority FROM background_jobs ORDER BY dedup_key")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        priorities,
        [
            ("followings:42:public".into(), 20),
            ("liked-tracks:42:owner".into(), 25),
            ("liked-tracks:43:owner".into(), 20),
            ("liked-tracks:44:owner".into(), 20),
            ("track-favoriters:42:public".into(), 15)
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn an_owner_read_outranks_bulk_work_and_lifts_a_queued_refresh(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    ensure(&pool, CatalogCollection::LikedTracks, "42", false, 600).await?;
    ensure(&pool, CatalogCollection::LikedTracks, "42", true, 600).await?;
    let priorities: Vec<(String, i16)> =
        sqlx::query_as("SELECT dedup_key, priority FROM background_jobs ORDER BY dedup_key")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        priorities,
        [
            ("liked-tracks:42:owner".into(), 20),
            ("liked-tracks:42:public".into(), 15)
        ]
    );
    sqlx::query("UPDATE background_jobs SET priority = 5")
        .execute(&pool)
        .await?;
    ensure(&pool, CatalogCollection::LikedTracks, "42", true, 600).await?;
    let owner: (i16, i64) = sqlx::query_as(
        "SELECT priority, generation FROM background_jobs WHERE dedup_key = 'liked-tracks:42:owner'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(owner, (20, 1));
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_collection_that_failed_for_good_is_held_for_six_hours(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    fail_for_good(
        &pool,
        "liked-tracks:42:public",
        "not_found: SoundCloud API error (status 404)",
        "1 hour",
    )
    .await?;

    let sync = ensure(&pool, CatalogCollection::LikedTracks, "42", false, 600).await?;

    assert_eq!(sync.status, "refreshing");
    assert_eq!(sync.error_code, Some("not_found"));
    assert_eq!(sync.queued_since, None);
    assert!((17_990..=18_000).contains(&sync.retry_after_seconds));
    assert_eq!(queued(&pool).await?, 0);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn after_the_hold_the_collection_is_queued_again_and_still_reports_the_error(
    pool: PgPool,
) -> anyhow::Result<()> {
    install(&pool).await?;
    fail_for_good(
        &pool,
        "liked-tracks:42:public",
        "forbidden: SoundCloud API error (status 403)",
        "8 hours",
    )
    .await?;
    fail_for_good(
        &pool,
        "liked-tracks:42:public",
        "not_found: SoundCloud API error (status 404)",
        "7 hours",
    )
    .await?;

    let sync = ensure(&pool, CatalogCollection::LikedTracks, "42", false, 600).await?;

    assert_eq!(sync.error_code, Some("not_found"));
    assert!(sync.queued_since.is_some());
    assert_eq!(sync.retry_after_seconds, 5);
    assert_eq!(queued(&pool).await?, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_successful_pass_clears_the_error(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    fail_for_good(
        &pool,
        "followings:42:owner",
        "not_found: SoundCloud API error (status 404)",
        "3 hours",
    )
    .await?;
    sqlx::query(
        "INSERT INTO catalog_collection_sync (subject_id, collection, scope, job_id, generation, snapshot_id, synced_at, complete)
         VALUES ('42', 'followings', 'owner', $1, 1, $2, now() - interval '1 hour', true)",
    )
    .bind(Uuid::now_v7())
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await?;

    let sync = ensure(&pool, CatalogCollection::Followings, "42", true, 600).await?;

    assert_eq!(sync.status, "refreshing");
    assert!(sync.last_completed_at.is_some());
    assert_eq!(sync.error_code, None);
    assert_eq!(queued(&pool).await?, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn auth_required_waits_for_a_new_sign_in(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    sqlx::query(
        "INSERT INTO soundcloud_connections (soundcloud_user_id, created_at)
         VALUES ('42', now() - interval '40 days')",
    )
    .execute(&pool)
    .await?;
    fail_for_good(
        &pool,
        "liked-playlists:42:owner",
        "auth_required: owner must sign in again",
        "1 hour",
    )
    .await?;

    for age in ["1 hour", "29 days"] {
        sqlx::query("UPDATE background_job_failures SET failed_at = now() - $1::interval")
            .bind(age)
            .execute(&pool)
            .await?;
        let sync = ensure(&pool, CatalogCollection::LikedPlaylists, "42", true, 600).await?;
        assert_eq!(sync.error_code, Some("auth_required"));
        assert_eq!(sync.retry_after_seconds, 0);
        assert_eq!(queued(&pool).await?, 0);
    }

    sqlx::query(
        "INSERT INTO soundcloud_connections (soundcloud_user_id) VALUES ('soundcloud:users:42')",
    )
    .execute(&pool)
    .await?;
    let sync = ensure(&pool, CatalogCollection::LikedPlaylists, "42", true, 600).await?;

    assert_eq!(sync.error_code, None);
    assert!(sync.queued_since.is_some());
    assert_eq!(queued(&pool).await?, 1);
    Ok(())
}

#[sqlx::test(migrations = false)]
async fn a_queued_collection_reports_since_when(pool: PgPool) -> anyhow::Result<()> {
    install(&pool).await?;
    let first = ensure(&pool, CatalogCollection::LikedTracks, "42", true, 600).await?;
    let created_at: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "UPDATE background_jobs SET created_at = now() - interval '20 minutes' RETURNING created_at",
    )
    .fetch_one(&pool)
    .await?;

    let again = ensure(&pool, CatalogCollection::LikedTracks, "42", true, 600).await?;

    assert!(first.queued_since.is_some());
    assert_eq!(again.queued_since, Some(created_at));
    assert_eq!(again.error_code, None);
    assert_eq!(queued(&pool).await?, 1);
    Ok(())
}
