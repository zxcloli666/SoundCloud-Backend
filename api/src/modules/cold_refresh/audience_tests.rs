use sqlx::PgPool;

use super::{
    ColdRefreshService, PLAYLIST_REPOSTERS, TRACK_FAVORITERS, TRACK_REPOSTERS, collection::ensure,
    read_audience_page,
};

fn service(pool: &PgPool) -> std::sync::Arc<ColdRefreshService> {
    ColdRefreshService::new(
        pool.clone(),
        crate::config::ColdCfg {
            track_ttl_sec: 3600,
            user_ttl_sec: 3600,
            playlist_ttl_sec: 3600,
            liked_tracks_ttl_sec: 3600,
            liked_playlists_ttl_sec: 3600,
            followings_ttl_sec: 3600,
            owned_ttl_sec: 3600,
            evict_after_sec: 86400,
        },
    )
}

async fn queued(pool: &PgPool, kind: &str) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar(
        "SELECT dedup_key FROM background_jobs WHERE kind = $1 ORDER BY dedup_key",
    )
    .bind(kind)
    .fetch_all(pool)
    .await?)
}

async fn seed_users(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized)
         SELECT n::text, 'soundcloud:users:' || n, 'User ' || n, 'user ' || n
         FROM generate_series(1, 3) n;",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn each_relation_reads_only_its_own_audience(pool: PgPool) -> anyhow::Result<()> {
    seed_users(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO catalog_audience (subject_urn, relation, user_urn, created_at) VALUES
         ('soundcloud:tracks:42', 'track-favoriters', 'soundcloud:users:1', now()),
         ('soundcloud:tracks:42', 'track-reposters', 'soundcloud:users:2', now()),
         ('soundcloud:playlists:42', 'playlist-reposters', 'soundcloud:users:3', now());",
    )
    .execute(&pool)
    .await?;
    let favoriters = read_audience_page(&pool, &TRACK_FAVORITERS, "42", 0, 50).await?;
    assert_eq!(favoriters.collection.len(), 1);
    assert_eq!(favoriters.collection[0]["urn"], "soundcloud:users:1");
    let reposters = read_audience_page(&pool, &TRACK_REPOSTERS, "42", 0, 50).await?;
    assert_eq!(reposters.collection[0]["urn"], "soundcloud:users:2");
    let playlist = read_audience_page(&pool, &PLAYLIST_REPOSTERS, "42", 0, 50).await?;
    assert_eq!(playlist.collection[0]["urn"], "soundcloud:users:3");
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_audience_member_without_a_catalog_profile_is_skipped(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_users(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO catalog_audience (subject_urn, relation, user_urn, created_at) VALUES
         ('soundcloud:tracks:42', 'track-favoriters', 'soundcloud:users:404', now()),
         ('soundcloud:tracks:42', 'track-favoriters', 'soundcloud:users:1', now() - interval '1 minute'),
         ('soundcloud:tracks:42', 'track-favoriters', 'soundcloud:users:2', now() - interval '2 minutes');",
    )
    .execute(&pool)
    .await?;
    let page = read_audience_page(&pool, &TRACK_FAVORITERS, "soundcloud:tracks:42", 0, 50).await?;
    assert_eq!(page.collection.len(), 2);
    assert_eq!(page.collection[0]["urn"], "soundcloud:users:1");
    assert_eq!(page.collection[1]["urn"], "soundcloud:users:2");
    assert!(!page.has_more);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn the_last_allowed_audience_page_does_not_advertise_an_unreachable_next_page(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized)
         SELECT n::text, 'soundcloud:users:' || n, 'User ' || n, 'user ' || n
         FROM generate_series(1, 102) n;
         INSERT INTO catalog_audience (subject_urn, relation, user_urn)
         SELECT 'soundcloud:tracks:42', 'track-reposters', 'soundcloud:users:' || n
         FROM generate_series(1, 102) n;",
    )
    .execute(&pool)
    .await?;
    let penultimate = read_audience_page(&pool, &TRACK_REPOSTERS, "42", 99, 1).await?;
    assert!(penultimate.has_more);
    let last = read_audience_page(&pool, &TRACK_REPOSTERS, "42", 100, 1).await?;
    assert_eq!(last.collection.len(), 1);
    assert!(!last.has_more);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_audience_refresh_is_public_and_deduplicated(pool: PgPool) -> anyhow::Result<()> {
    let refresh = async |coll: super::AudienceCollection, urn: &str| {
        ensure(&pool, coll.kind, urn, false, coll.ttl_sec).await
    };
    let sync = refresh(TRACK_FAVORITERS, "soundcloud:tracks:42").await?;
    assert_eq!(sync.status, "refreshing");
    refresh(TRACK_FAVORITERS, "42").await?;
    refresh(PLAYLIST_REPOSTERS, "soundcloud:playlists:42").await?;
    let jobs: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT payload FROM background_jobs WHERE kind = 'catalog.collection' ORDER BY dedup_key",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(
        jobs,
        [
            serde_json::json!({"version":"1","payload":{"collection":"playlist-reposters","subject_id":"42","owner":false}}),
            serde_json::json!({"version":"1","payload":{"collection":"track-favoriters","subject_id":"42","owner":false}})
        ]
    );
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn an_unknown_subject_asks_for_the_entity_before_its_audience(
    pool: PgPool,
) -> anyhow::Result<()> {
    let page = service(&pool)
        .audience_page(TRACK_FAVORITERS, "soundcloud:tracks:42", 0, 30)
        .await?;
    assert!(page.collection.is_empty());
    assert_eq!(page.sync.status, "refreshing");
    assert!(page.sync.last_completed_at.is_none());
    assert!(queued(&pool, "catalog.collection").await?.is_empty());
    assert_eq!(queued(&pool, "catalog.refresh").await?, ["track:42:public"]);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_subject_that_stopped_being_public_serves_and_keeps_nothing(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_users(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing)
         VALUES ('42', 'soundcloud:tracks:42', 'Hidden', 'hidden', 1000, 'private');
         INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, sharing)
         VALUES ('42', 'soundcloud:playlists:42', 'Set', 'set', 'public');
         INSERT INTO catalog_audience (subject_urn, relation, user_urn) VALUES
         ('soundcloud:tracks:42', 'track-favoriters', 'soundcloud:users:1'),
         ('soundcloud:tracks:42', 'track-reposters', 'soundcloud:users:2'),
         ('soundcloud:playlists:42', 'playlist-reposters', 'soundcloud:users:3');
         INSERT INTO catalog_collection_sync (subject_id, collection, scope, job_id, generation, snapshot_id, synced_at, complete) VALUES
         ('42', 'track-favoriters', 'public', gen_random_uuid(), 1, gen_random_uuid(), now(), true),
         ('42', 'playlist-reposters', 'public', gen_random_uuid(), 1, gen_random_uuid(), now(), true);",
    )
    .execute(&pool)
    .await?;
    let page = service(&pool)
        .audience_page(TRACK_FAVORITERS, "42", 0, 30)
        .await?;
    assert!(page.collection.is_empty());
    assert_eq!(page.sync.status, "ready");
    assert_eq!(page.sync.retry_after_seconds, 0);
    assert!(queued(&pool, "catalog.collection").await?.is_empty());
    assert!(queued(&pool, "catalog.refresh").await?.is_empty());
    let left: Vec<String> = sqlx::query_scalar("SELECT subject_urn FROM catalog_audience")
        .fetch_all(&pool)
        .await?;
    assert_eq!(left, ["soundcloud:playlists:42"]);
    let snapshots: Vec<String> =
        sqlx::query_scalar("SELECT collection FROM catalog_collection_sync ORDER BY collection")
            .fetch_all(&pool)
            .await?;
    assert_eq!(snapshots, ["playlist-reposters"]);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_public_subject_serves_its_mirror_and_schedules_one_refresh(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed_users(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO playlists (sc_playlist_id, urn, title, title_normalized, sharing)
         VALUES ('42', 'soundcloud:playlists:42', 'Set', 'set', 'public');
         INSERT INTO catalog_audience (subject_urn, relation, user_urn) VALUES
         ('soundcloud:playlists:42', 'playlist-reposters', 'soundcloud:users:1');",
    )
    .execute(&pool)
    .await?;
    let page = service(&pool)
        .audience_page(PLAYLIST_REPOSTERS, "soundcloud:playlists:42", 0, 30)
        .await?;
    assert_eq!(page.collection.len(), 1);
    assert_eq!(page.collection[0]["urn"], "soundcloud:users:1");
    assert_eq!(page.sync.status, "refreshing");
    assert_eq!(
        queued(&pool, "catalog.collection").await?,
        ["playlist-reposters:42:public"]
    );
    assert!(queued(&pool, "catalog.refresh").await?.is_empty());
    Ok(())
}
