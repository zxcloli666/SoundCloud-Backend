use uuid::Uuid;

use super::{UNHEALTHY_APPS_MAX, worst_unhealthy};
use crate::modules::auth::health::AppHealth;

#[test]
fn the_login_sends_the_worst_unhealthy_apps_when_there_are_too_many() {
    let healths: Vec<AppHealth> = (0..150)
        .map(|n| AppHealth {
            successes: 150 - n,
            failures: 100 + n,
        })
        .collect();
    let ids: Vec<Uuid> = (0..150u128).map(Uuid::from_u128).collect();
    let unhealthy = ids.iter().copied().zip(healths.iter()).collect();

    let sent = worst_unhealthy(unhealthy);

    assert_eq!(sent.len(), UNHEALTHY_APPS_MAX);
    assert_eq!(sent.first(), Some(&Uuid::from_u128(149)));
    assert!(
        !sent.contains(&Uuid::from_u128(0)),
        "the least failing apps are the ones left out"
    );
    assert_eq!(sent.last(), Some(&Uuid::from_u128(50)));
}
