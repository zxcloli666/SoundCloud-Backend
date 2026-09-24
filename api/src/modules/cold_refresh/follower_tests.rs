use sqlx::PgPool;

use super::{
    FOLLOWERS,
    collection::{CollectionPage, ensure},
    read_collection_page,
};

#[sqlx::test(migrations = "./migrations")]
async fn collection_last_allowed_page_does_not_advertise_an_unreachable_next_page(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::raw_sql("INSERT INTO users (sc_user_id, urn, username, username_normalized)
        SELECT n::text, 'soundcloud:users:' || n, 'Follower', 'follower' FROM generate_series(1, 102) n;
        INSERT INTO user_followers (user_id, target_user_urn)
        SELECT '42', 'soundcloud:users:' || n FROM generate_series(1, 102) n;")
        .execute(&pool).await?;
    let penultimate = read_collection_page(&pool, &FOLLOWERS, "42", 99, 1, true).await?;
    assert!(penultimate.has_more);
    let last = read_collection_page(&pool, &FOLLOWERS, "42", 100, 1, true).await?;
    assert_eq!(last.collection.len(), 1);
    assert!(!last.has_more);
    Ok(())
}

#[sqlx::test(migrations = "./migrations")]
async fn followers_read_the_local_snapshot_and_schedule_one_public_refresh(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::raw_sql("INSERT INTO users (sc_user_id, urn, username, username_normalized) VALUES
        ('17', 'soundcloud:users:17', 'First', 'first'), ('18', 'soundcloud:users:18', 'Second', 'second');
        INSERT INTO user_followers (user_id, target_user_urn, created_at) VALUES
        ('42', 'soundcloud:users:99', now()),
        ('42', 'soundcloud:users:17', now() - interval '1 minute'),
        ('42', 'soundcloud:users:18', now() - interval '2 minutes');")
        .execute(&pool).await?;
    let sync = ensure(
        &pool,
        backend_contracts::CatalogCollection::Followers,
        "42",
        false,
        600,
    )
    .await?;
    let page = read_collection_page(&pool, &FOLLOWERS, "soundcloud:users:42", 0, 1, true).await?;
    assert_eq!(page.collection.len(), 1);
    assert_eq!(page.collection[0]["urn"], "soundcloud:users:17");
    assert!(page.has_more);
    assert_eq!(sync.status, "refreshing");
    let wire = serde_json::to_value(CollectionPage::new(page, sync))?;
    assert_eq!(wire["pageSize"], 1);
    assert_eq!(wire["hasMore"], true);
    assert!(wire.get("page_size").is_none());
    ensure(
        &pool,
        backend_contracts::CatalogCollection::Followers,
        "soundcloud:users:42",
        false,
        600,
    )
    .await?;
    let jobs: Vec<serde_json::Value> =
        sqlx::query_scalar("SELECT payload FROM background_jobs WHERE kind = 'catalog.collection'")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        jobs,
        [
            serde_json::json!({"version":"1","payload":{"collection":"followers","subject_id":"42","owner":false}})
        ]
    );
    let last = read_collection_page(&pool, &FOLLOWERS, "42", 1, 1, true).await?;
    assert_eq!(last.collection[0]["urn"], "soundcloud:users:18");
    assert!(!last.has_more);
    assert!(
        read_collection_page(&pool, &FOLLOWERS, "43", 0, 50, true)
            .await?
            .collection
            .is_empty()
    );
    Ok(())
}
