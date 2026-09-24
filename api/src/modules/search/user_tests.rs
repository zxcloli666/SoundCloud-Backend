use super::SearchService;
use crate::cache::CacheService;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrations = "./migrations")]
async fn user_search_is_local_bounded_and_reads_current_profiles(
    pool: PgPool,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO users (sc_user_id, urn, username, username_normalized, followers_count)
        VALUES ('17', 'soundcloud:users:17', 'Alice', 'alice', 10),
               ('18', 'soundcloud:users:18', 'Alpine', 'alpine', 20),
               ('19', 'soundcloud:users:19', 'Bob', 'bob', 30)",
    )
    .execute(&pool)
    .await?;
    let redis = deadpool_redis::Config::from_url("redis://127.0.0.1:1")
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))?;
    let service = SearchService::new(pool.clone(), CacheService::new(redis));

    let first = service.users("AL", None, 0, 1).await?;
    assert_eq!(first.collection[0]["username"], "Alpine");
    assert!(first.has_more);
    let second = service.users("al", None, 1, 1).await?;
    assert_eq!(second.collection[0]["username"], "Alice");
    assert!(!second.has_more);

    let combined = service.users("al", Some("19,17"), 0, 30).await?;
    assert_eq!(combined.collection.len(), 1);
    assert_eq!(combined.collection[0]["username"], "Alice");
    let ids = service
        .users("", Some("19,017,soundcloud:users:19,999"), 0, 30)
        .await?;
    let names: Vec<_> = ids
        .collection
        .iter()
        .map(|v| v["username"].clone())
        .collect();
    assert_eq!(names, vec![json!("Bob"), json!("Alice")]);
    assert!(!ids.has_more);

    let empty = service.users("", None, -5, 200).await?;
    assert!(empty.collection.is_empty());
    assert_eq!((empty.page, empty.page_size), (0, 50));
    assert!(
        service
            .users("a", Some("17"), 0, 30)
            .await?
            .collection
            .is_empty()
    );
    assert!(
        service
            .users("al", Some("soundcloud:tracks:17"), 0, 30)
            .await
            .is_err()
    );

    sqlx::query("UPDATE users SET username = 'After login', username_normalized = 'after login' WHERE sc_user_id = '17'")
        .execute(&pool).await?;
    let current = service.users("", Some("17"), 0, 30).await?;
    assert_eq!(current.collection[0]["username"], "After login");
    Ok(())
}
