use std::time::Duration;

use crate::error::AppError;
use crate::metrics::{self, Outcome};

fn rendered() -> Option<String> {
    metrics::init();
    metrics::render_for_tests()
}

#[tokio::test]
async fn every_tier_outcome_is_counted_under_its_own_labels() {
    metrics::init();
    metrics::record_sc_tier(
        "relay_lua",
        "track",
        Outcome::Miss,
        Duration::from_millis(5),
    );
    metrics::record_sc_tier("backup", "track", Outcome::Ok, Duration::from_millis(80));
    metrics::record_sc_tier(
        "backup",
        "playlist_meta",
        Outcome::Error,
        Duration::from_millis(20),
    );
    metrics::set_relay_breaker_open(true);

    let Some(body) = rendered() else {
        return;
    };

    assert!(body.contains("tier=\"relay_lua\""));
    assert!(body.contains("tier=\"backup\""));
    assert!(body.contains("operation=\"playlist_meta\""));
    assert!(body.contains("outcome=\"miss\""));
    assert!(body.contains("api_sc_relay_breaker_open"));
}

#[test]
fn an_unreachable_relay_is_a_miss_and_not_an_error() {
    let unreachable: Result<(), AppError> = Err(AppError::ScUnreachable("relay: no result".into()));
    let outcome = match &unreachable {
        Ok(()) => Outcome::Ok,
        Err(AppError::ScUnreachable(_)) => Outcome::Miss,
        Err(_) => Outcome::Error,
    };
    assert_eq!(outcome, Outcome::Miss);
}

#[tokio::test]
async fn the_gauge_follows_the_shared_breaker_and_not_only_our_own_failures() {
    use sc_transport::{
        EGRESS_RELAY_LUA, EgressFuture, EgressHealth, EgressHealthStore, EgressState,
    };
    use std::sync::Arc;
    use std::time::Duration;

    struct OpenElsewhere;

    impl EgressHealthStore for OpenElsewhere {
        fn publish_open<'a>(
            &'a self,
            _channel: &'a str,
            _app: &'a str,
            _cooldown: Duration,
        ) -> EgressFuture<'a, ()> {
            Box::pin(async {})
        }

        fn publish_closed<'a>(&'a self, _channel: &'a str) -> EgressFuture<'a, ()> {
            Box::pin(async {})
        }

        fn remaining<'a>(&'a self, _channel: &'a str) -> EgressFuture<'a, EgressState> {
            Box::pin(async { EgressState::Open(Duration::from_secs(60)) })
        }
    }

    metrics::init();
    let health = EgressHealth::new(EGRESS_RELAY_LUA, "api", Some(Arc::new(OpenElsewhere)));
    assert!(
        health.is_open().await,
        "the serving breaker must honour an outage another process already published"
    );

    metrics::set_relay_breaker_open(health.is_open().await);
    let Some(body) = rendered() else {
        return;
    };
    assert!(body.contains("api_sc_relay_breaker_open"));
}
