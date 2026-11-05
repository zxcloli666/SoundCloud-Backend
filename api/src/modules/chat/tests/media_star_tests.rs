use sqlx::PgPool;

use super::Stars;
use crate::modules::subscriptions::SubscriptionsService;

const FAR: i64 = 4_000_000_000;

#[test]
fn a_yes_lives_sixty_seconds_and_a_no_fifteen() {
    use crate::modules::chat::media::limits::{STAR_NO_TTL, STAR_YES_TTL};
    assert_eq!(STAR_YES_TTL.as_secs(), 60);
    assert_eq!(STAR_NO_TTL.as_secs(), 15);
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_no_is_cached_so_a_fresh_purchase_waits_for_the_ttl(pool: PgPool) {
    let service = SubscriptionsService::new(pool.clone(), false);
    let stars = Stars::new(service.clone());
    assert!(!stars.is_star("301").await.expect("an answer"));
    service.upsert("301", FAR, None).await.expect("a purchase");
    assert!(!stars.is_star("301").await.expect("a cached answer"));
    let fresh = Stars::new(service);
    assert!(fresh.is_star("301").await.expect("an answer"));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn a_yes_is_cached_and_a_lapsed_subscription_is_a_no(pool: PgPool) {
    let service = SubscriptionsService::new(pool.clone(), false);
    service.upsert("302", FAR, Some(1)).await.expect("a star");
    let stars = Stars::new(service.clone());
    assert!(stars.is_star("302").await.expect("an answer"));
    service.remove("302").await.expect("removed");
    assert!(stars.is_star("302").await.expect("a cached answer"));
    service.upsert("303", 1, None).await.expect("a lapsed one");
    assert!(!Stars::new(service).is_star("303").await.expect("an answer"));
}

#[sqlx::test(migrator = "crate::db::TEST_MIGRATOR")]
async fn the_always_premium_switch_makes_everyone_a_star(pool: PgPool) {
    let stars = Stars::new(SubscriptionsService::new(pool, true));
    assert!(stars.is_star("999").await.expect("an answer"));
}
