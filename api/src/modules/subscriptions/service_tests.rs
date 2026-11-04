use sqlx::PgPool;

use super::service::SubscriptionsService;

const DAY: i64 = 86_400;

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
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
