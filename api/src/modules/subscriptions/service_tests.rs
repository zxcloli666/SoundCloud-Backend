use sqlx::PgPool;

use super::service::SubscriptionsService;

const DAY: i64 = 86_400;

#[sqlx::test(migrations = "./migrations")]
async fn status_carries_star_since_only_while_premium(pool: PgPool) {
    let service = SubscriptionsService::new(pool, false);
    let now = chrono::Utc::now().timestamp();
    service
        .upsert("soundcloud:users:1", now + 10 * DAY, Some(now - 200 * DAY))
        .await
        .unwrap();
    service
        .upsert("soundcloud:users:2", now - DAY, Some(now - 90 * DAY))
        .await
        .unwrap();
    service
        .upsert("soundcloud:users:3", now + DAY, None)
        .await
        .unwrap();

    let active = service.status("soundcloud:users:1").await.unwrap();
    assert!(active.premium);
    assert_eq!(active.star_since, Some(now - 200 * DAY));

    let lapsed = service.status("soundcloud:users:2").await.unwrap();
    assert!(!lapsed.premium);
    assert_eq!(lapsed.star_since, None);

    let unknown = service.status("soundcloud:users:9").await.unwrap();
    assert!(!unknown.premium);

    assert_eq!(
        service
            .status("soundcloud:users:3")
            .await
            .unwrap()
            .star_since,
        None
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn the_cached_star_answer_skips_the_database_until_a_write(pool: PgPool) {
    let service = SubscriptionsService::new(pool.clone(), false);
    let now = chrono::Utc::now().timestamp();
    assert!(
        !service
            .is_premium_cached("soundcloud:users:7")
            .await
            .unwrap()
    );
    service
        .upsert("soundcloud:users:7", now + DAY, None)
        .await
        .unwrap();
    assert!(service.is_premium_cached("7").await.unwrap());
    sqlx::query("DELETE FROM subscriptions")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        service
            .is_premium_cached("soundcloud:users:7")
            .await
            .unwrap(),
        "a cached answer does not read the database again"
    );
    assert!(!service.is_premium("soundcloud:users:7").await.unwrap());
    service.remove("soundcloud:users:7").await.unwrap();
    assert!(!service.is_premium_cached("7").await.unwrap());
}
