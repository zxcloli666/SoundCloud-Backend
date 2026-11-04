use std::time::Duration;

use rand::SeedableRng;
use rand::rngs::StdRng;

use super::retry_wait;
use crate::api::CallError;
use crate::transport::Failure;

#[test]
fn retries_follow_the_desktop_client_loop() {
    let mut rng = StdRng::seed_from_u64(3);
    let clock = CallError::Inner {
        status: 401,
        code: "chat_clock".to_owned(),
        retry_ms: None,
    };
    assert_eq!(retry_wait(&clock, 1, &mut rng), Duration::ZERO);
    let busy = CallError::Inner {
        status: 503,
        code: "chat_busy".to_owned(),
        retry_ms: Some(1_500),
    };
    assert_eq!(retry_wait(&busy, 4, &mut rng), Duration::from_millis(1_500));
    let full = CallError::Outer {
        status: 503,
        retry_after: Some(Duration::from_secs(1)),
    };
    assert_eq!(retry_wait(&full, 2, &mut rng), Duration::from_secs(1));
    let drained = CallError::Outer {
        status: 503,
        retry_after: None,
    };
    assert!(retry_wait(&drained, 1, &mut rng) <= Duration::from_secs(10));
    let lost = CallError::Transport(Failure::Connect);
    for failures in 1..=10 {
        let wait = retry_wait(&lost, failures, &mut rng);
        assert!(wait >= Duration::from_millis(500), "{wait:?}");
        assert!(wait <= Duration::from_secs(23), "{wait:?}");
    }
}
