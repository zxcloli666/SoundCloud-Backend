use sqlx::PgPool;

use super::{read_page, record_pending, submitted};

async fn seed(pool: &PgPool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized) VALUES
         ('1', 'soundcloud:users:1', 'First', 'first'),
         ('2', 'soundcloud:users:2', 'Second', 'second');
         INSERT INTO tracks (sc_track_id, urn, title, title_normalized, duration_ms, sharing)
         VALUES ('42', 'soundcloud:tracks:42', 'Song', 'song', 1000, 'public');",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[test]
fn a_submitted_comment_is_bounded_and_keeps_its_position() {
    let parsed = submitted(&serde_json::json!({"comment": {"body": " hi ", "timestamp": 1500}}))
        .expect("comment");
    assert_eq!(parsed.body, "hi");
    assert_eq!(parsed.track_position_ms, Some(1500));
    let flat = submitted(&serde_json::json!({"body": "hi"})).expect("comment");
    assert_eq!(flat.track_position_ms, None);
    for body in [
        serde_json::json!({"comment": {"body": "   "}}),
        serde_json::json!({"comment": {}}),
        serde_json::json!({"comment": {"body": "hi", "timestamp": -1}}),
        serde_json::json!({"comment": {"body": "hi", "timestamp": "1500"}}),
        serde_json::json!({"comment": {"body": "x".repeat(16 * 1024 + 1)}}),
    ] {
        assert!(submitted(&body).is_err(), "{body}");
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn a_pending_comment_is_readable_before_soundcloud_confirms_it(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    let comment = submitted(&serde_json::json!({"comment": {"body": "first", "timestamp": 250}}))?;
    let mut connection = pool.acquire().await?;
    record_pending(&mut connection, "42", "soundcloud:users:1", &comment).await?;
    drop(connection);
    let page = read_page(&pool, "42", 0, 30).await?;
    assert_eq!(page.collection.len(), 1);
    let card = &page.collection[0];
    assert_eq!(card["body"], "first");
    assert_eq!(card["pending"], true);
    assert_eq!(card["id"], serde_json::Value::Null);
    assert_eq!(card["timestamp"], 250);
    assert_eq!(card["track_id"], 42);
    assert_eq!(card["user_id"], 1);
    assert_eq!(card["user"]["urn"], "soundcloud:users:1");
    assert!(
        card["urn"]
            .as_str()
            .is_some_and(|urn| urn.starts_with("local:comments:"))
    );
    assert!(!page.has_more);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn confirmed_comments_come_first_and_authors_outside_the_catalog_are_skipped(
    pool: PgPool,
) -> anyhow::Result<()> {
    seed(&pool).await?;
    sqlx::raw_sql(
        "INSERT INTO track_comments (id, sc_track_id, sc_comment_id, user_urn, body, created_at) VALUES
         (gen_random_uuid(), '42', '7', 'soundcloud:users:1', 'newest', now()),
         (gen_random_uuid(), '42', '8', 'soundcloud:users:2', 'older', now() - interval '1 minute'),
         (gen_random_uuid(), '42', '9', 'soundcloud:users:404', 'unknown author', now() - interval '2 minutes');",
    )
    .execute(&pool)
    .await?;
    let first = read_page(&pool, "42", 0, 1).await?;
    assert_eq!(first.collection.len(), 1);
    assert_eq!(first.collection[0]["body"], "newest");
    assert_eq!(first.collection[0]["id"], 7);
    assert_eq!(first.collection[0]["pending"], false);
    assert_eq!(first.collection[0]["urn"], "soundcloud:comments:7");
    assert!(first.has_more);
    let second = read_page(&pool, "42", 1, 1).await?;
    assert_eq!(second.collection[0]["body"], "older");
    assert!(!second.has_more);
    let all = read_page(&pool, "42", 0, 30).await?;
    assert_eq!(all.collection.len(), 2);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn a_private_track_serves_no_comments_and_keeps_none(pool: PgPool) -> anyhow::Result<()> {
    seed(&pool).await?;
    sqlx::raw_sql(
        "UPDATE tracks SET sharing = 'private' WHERE sc_track_id = '42';
         INSERT INTO track_comments (id, sc_track_id, sc_comment_id, user_urn, body)
         VALUES (gen_random_uuid(), '42', '7', 'soundcloud:users:1', 'secret');
         INSERT INTO catalog_collection_sync (subject_id, collection, scope, job_id, generation, snapshot_id, synced_at, complete)
         VALUES ('42', 'track-comments', 'public', gen_random_uuid(), 1, gen_random_uuid(), now(), true);",
    )
    .execute(&pool)
    .await?;
    let service = crate::modules::cold_refresh::ColdRefreshService::new(
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
    );
    let page = service.comments_page("42", 0, 30).await?;
    assert!(page.collection.is_empty());
    assert_eq!(page.sync.status, "ready");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM track_comments")
        .fetch_one(&pool)
        .await?;
    assert_eq!(left, 0);
    let snapshots: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog_collection_sync")
        .fetch_one(&pool)
        .await?;
    assert_eq!(snapshots, 0);
    Ok(())
}
