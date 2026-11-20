use super::*;
use crate::modules::me::MeService;

async fn service(pool: &PgPool) -> anyhow::Result<std::sync::Arc<MeService>> {
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let cold = crate::modules::cold_refresh::ColdRefreshService::new(
        pool.clone(),
        crate::config::ColdCfg {
            track_ttl_sec: 3600,
            user_ttl_sec: 3600,
            playlist_ttl_sec: 3600,
            liked_tracks_ttl_sec: 3600,
            liked_playlists_ttl_sec: 3600,
            followings_ttl_sec: 3600,
            owned_ttl_sec: 300,
            library_sync_cooldown_sec: 21600,
            evict_after_sec: 86400,
        },
    );
    let miss = crate::modules::resolve::miss_tests::offline(pool).await?;
    Ok(MeService::new(
        pool.clone(),
        crate::modules::sync_queue::SyncQueueService::new(pool.clone(), redis),
        cold,
        crate::modules::users::PublicUsers::new(pool.clone(), miss),
    ))
}

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql("INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, uploader_sc_user_id, release_date)
        SELECT id::text, 'soundcloud:tracks:' || id, 'Track ' || id, 'track ' || id, 120000,
               CASE WHEN id = 5 THEN '18' ELSE '17' END, '2026-09-01'::date - id
        FROM generate_series(1, 7) id;
        UPDATE tracks SET sharing = 'private' WHERE sc_track_id = '2';
        UPDATE tracks SET deleted_at = now() WHERE sc_track_id = '3';
        UPDATE tracks SET superseded_by = (SELECT id FROM tracks WHERE sc_track_id = '1') WHERE sc_track_id = '4';
        UPDATE tracks SET uploader_sc_user_id = 'soundcloud:users:17' WHERE sc_track_id = '7';
        INSERT INTO user_followings (user_id, target_user_urn, wanted_state, progress) VALUES
            ('42', 'soundcloud:users:17', true, false),
            ('soundcloud:users:42', 'soundcloud:users:17', true, false),
            ('42', 'soundcloud:users:18', false, true),
            ('soundcloud:users:42', 'soundcloud:users:18', true, false);
        INSERT INTO user_likes_tracks (user_id, sc_track_id, wanted_state, progress) VALUES ('42', '1', true, false);")
        .execute(pool).await?;
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn local_feed_filters_before_pagination_and_prefers_current_follow_intent(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let first = read(&pool, "soundcloud:users:42", 0, 1).await?;
    assert_eq!(first.collection.len(), 1);
    assert_eq!(first.collection[0]["id"], 1);
    assert_eq!(first.collection[0]["user_favorite"], true);
    assert!(first.has_more);
    let second = read(&pool, "42", 1, 1).await?;
    assert_eq!(second.collection[0]["id"], 6);
    let last = read(&pool, "42", 2, 1).await?;
    assert_eq!(last.collection[0]["id"], 7);
    assert!(!last.has_more);
    assert!(read(&pool, "99", 0, 50).await?.collection.is_empty());
    let bounded = read(&pool, "42", i64::MAX, i64::MAX).await?;
    assert_eq!(
        (bounded.page, bounded.page_size, bounded.has_more),
        (24, 50, false)
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn feed_and_follow_changes_work_with_external_services_unavailable(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let me = service(&pool).await?;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        me.get_followings_tracks("42", 0, 50),
    )
    .await??;
    assert_eq!(result.page.collection.len(), 3);
    assert_eq!(result.followings_sync.status, "refreshing");
    assert!(
        serde_json::to_value(&result)?
            .get("followingsSync")
            .is_some()
    );
    me.get_followings_tracks("42", 0, 50).await?;
    me.follow_user("42", "18").await?;
    let followed = me.get_followings_tracks("42", 0, 50).await?;
    assert_eq!(followed.page.collection.len(), 4);
    let queued: Vec<String> = sqlx::query_scalar("SELECT dedup_key FROM background_jobs WHERE kind = 'catalog.collection' ORDER BY dedup_key")
        .fetch_all(&pool).await?;
    assert_eq!(queued, ["followings:42:owner", "owned-tracks:18:public"]);
    me.unfollow_user("42", "soundcloud:users:18").await?;
    assert_eq!(
        me.get_followings_tracks("42", 0, 50)
            .await?
            .page
            .collection
            .len(),
        3
    );
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_self_follow_is_refused_before_anything_is_written(pool: PgPool) -> anyhow::Result<()> {
    let me = service(&pool).await?;
    for target in ["42", "soundcloud:users:42"] {
        let error = me.follow_user("42", target).await.unwrap_err();
        assert_eq!(error.public_code(), "cannot_follow_self");
        assert_eq!(error.status(), axum::http::StatusCode::BAD_REQUEST);
    }
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM sync_queue")
        .fetch_one(&pool)
        .await?;
    let mirrored: i64 = sqlx::query_scalar("SELECT count(*) FROM user_followings")
        .fetch_one(&pool)
        .await?;
    assert_eq!((queued, mirrored), (0, 0));
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn follow_and_upload_refresh_enqueue_commit_together(pool: PgPool) -> anyhow::Result<()> {
    let me = service(&pool).await?;
    sqlx::raw_sql(
        "CREATE FUNCTION reject_test_upload_job() RETURNS trigger LANGUAGE plpgsql AS $$
        BEGIN RAISE EXCEPTION 'injected enqueue failure'; END $$;
        CREATE TRIGGER reject_test_upload_job BEFORE INSERT ON background_jobs
        FOR EACH ROW EXECUTE FUNCTION reject_test_upload_job();",
    )
    .execute(&pool)
    .await?;
    assert!(me.follow_user("42", "17").await.is_err());
    let rows: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM user_followings), (SELECT count(*) FROM sync_queue)",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(rows, (0, 0));
    assert!(me.follow_user("42", "soundcloud:tracks:17").await.is_err());
    Ok(())
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn feed_pages_merge_uploaders_in_one_order(pool: PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql("INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, uploader_sc_user_id, release_date)
        SELECT id::text, 'soundcloud:tracks:' || id, 'Track ' || id, 'track ' || id, 120000,
               CASE WHEN id % 3 = 0 THEN '18' ELSE '17' END,
               CASE WHEN id % 11 = 0 THEN NULL ELSE '2026-09-01'::date - (id * 7 % 40) END
        FROM generate_series(1, 45) id;
        INSERT INTO user_followings (user_id, target_user_urn, wanted_state, progress) VALUES
            ('42', 'soundcloud:users:17', true, false),
            ('42', 'soundcloud:users:18', true, false);")
        .execute(&pool)
        .await?;
    let expected: Vec<i64> = sqlx::query_scalar(
        "SELECT track.sc_track_id::bigint FROM tracks AS track
         ORDER BY track.release_date DESC NULLS LAST, track.sc_created_at DESC NULLS LAST,
                  track.sc_track_id DESC",
    )
    .fetch_all(&pool)
    .await?;
    let ids = |page: &ListPageResult<Value>| -> Vec<i64> {
        page.collection
            .iter()
            .filter_map(|t| t["id"].as_i64())
            .collect()
    };
    let whole = read(&pool, "42", 0, 50).await?;
    assert_eq!(ids(&whole), expected);
    assert!(!whole.has_more);
    let mut paged = Vec::new();
    for page in 0..7 {
        let part = read(&pool, "42", page, 7).await?;
        assert_eq!(part.has_more, page < 6);
        paged.extend(ids(&part));
    }
    assert_eq!(paged, expected);
    Ok(())
}
