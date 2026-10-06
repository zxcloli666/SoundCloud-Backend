use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use deadpool_redis::{Config, Runtime};
use sc_transport::{EgressFuture, EgressHealthStore, EgressState};

use super::gate::{BREAKER_CHANNEL, Closed, LiveGate, PAUSE_CHANNEL, cooling_seconds, paced};
use super::meta::LiveState;
use super::query::LiveClass;
use crate::common::admission::{Decision, PublicAdmission};
use crate::config::{AdmissionCfg, AdmissionLimitCfg};

#[derive(Default)]
pub(super) struct SharedEgress {
    open: Mutex<HashMap<String, Duration>>,
    published: Mutex<Vec<(String, Duration)>>,
}

impl SharedEgress {
    pub(super) fn opened(&self, channel: &str, cooldown: Duration) {
        self.open
            .lock()
            .unwrap()
            .insert(channel.to_owned(), cooldown);
    }

    fn published(&self) -> Vec<(String, Duration)> {
        self.published.lock().unwrap().clone()
    }
}

impl EgressHealthStore for SharedEgress {
    fn publish_open<'a>(
        &'a self,
        channel: &'a str,
        _app: &'a str,
        cooldown: Duration,
    ) -> EgressFuture<'a, ()> {
        self.published
            .lock()
            .unwrap()
            .push((channel.to_owned(), cooldown));
        self.opened(channel, cooldown);
        Box::pin(async {})
    }

    fn publish_closed<'a>(&'a self, channel: &'a str) -> EgressFuture<'a, ()> {
        self.open.lock().unwrap().remove(channel);
        Box::pin(async {})
    }

    fn remaining<'a>(&'a self, channel: &'a str) -> EgressFuture<'a, EgressState> {
        let state = match self.open.lock().unwrap().get(channel) {
            Some(left) => EgressState::Open(*left),
            None => EgressState::Closed,
        };
        Box::pin(async move { state })
    }
}

struct UnreachableEgress;

impl EgressHealthStore for UnreachableEgress {
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
        Box::pin(async { EgressState::Closed })
    }
}

pub(super) fn admission(redis_url: &str, limit: AdmissionLimitCfg) -> Arc<PublicAdmission> {
    let pool = Config::from_url(redis_url)
        .create_pool(Some(Runtime::Tokio1))
        .expect("a redis pool builds without connecting");
    PublicAdmission::for_live_search(
        pool,
        AdmissionCfg {
            window: Duration::from_secs(60),
            timeout: Duration::from_millis(200),
            max_in_flight: 16,
            login: limit,
            link_create: limit,
            resolve: limit,
            live_main: limit,
            live_side: limit,
            live_import: limit,
            live_rescue: limit,
            live_proxy: limit,
            live_entity: limit,
        },
    )
}

fn offline_gate(store: Arc<dyn EgressHealthStore>, in_flight: usize) -> LiveGate {
    let limit = AdmissionLimitCfg {
        per_client: 10,
        global: 10,
    };
    LiveGate::new(store, admission("redis://127.0.0.1:1", limit), in_flight)
}

fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_owned())
}

fn identity() -> String {
    format!("gate-test-{}", uuid::Uuid::now_v7())
}

async fn state_of(gate: &LiveGate, class: LiveClass) -> Option<LiveState> {
    gate.open(class, &identity())
        .await
        .err()
        .map(|closed| closed.state)
}

#[tokio::test]
async fn the_pause_row_beats_every_other_gate() {
    let store = Arc::new(SharedEgress::default());
    store.opened(PAUSE_CHANNEL, Duration::from_secs(3600));
    store.opened(BREAKER_CHANNEL, Duration::from_secs(60));
    let gate = offline_gate(store, 0);

    assert_eq!(
        gate.open(LiveClass::Main, "anyone").await.err(),
        Some(Closed::new(LiveState::Paused, 3600)),
        "a pause says how long it still holds"
    );
}

#[tokio::test]
async fn an_open_breaker_beats_admission_and_the_in_flight_cap() {
    let store = Arc::new(SharedEgress::default());
    store.opened(BREAKER_CHANNEL, Duration::from_secs(60));
    let gate = offline_gate(store, 0);

    assert_eq!(
        state_of(&gate, LiveClass::Main).await,
        Some(LiveState::Cooling)
    );
}

#[tokio::test]
async fn an_unreachable_admission_store_keeps_soundcloud_shut() {
    let gate = offline_gate(Arc::new(SharedEgress::default()), 8);

    assert_eq!(
        state_of(&gate, LiveClass::Main).await,
        Some(LiveState::Busy),
        "without its budget store live search must not go out at all"
    );
    assert!(!gate.proxy_admits().await);
}

#[tokio::test]
async fn four_silent_searches_open_the_breaker_for_both_nodes() {
    let store = Arc::new(SharedEgress::default());
    let gate = offline_gate(store.clone(), 8);

    for _ in 0..3 {
        gate.record_silence().await;
    }
    assert_eq!(
        state_of(&gate, LiveClass::Main).await,
        Some(LiveState::Busy)
    );
    gate.record_silence().await;

    assert_eq!(
        gate.open(LiveClass::Main, &identity()).await.err(),
        Some(Closed::new(LiveState::Cooling, 60))
    );
    assert_eq!(
        store.published(),
        [(BREAKER_CHANNEL.to_owned(), Duration::from_secs(60))],
        "the trip is published under search_live and nowhere else"
    );

    gate.record_answer().await;
    assert_eq!(
        state_of(&gate, LiveClass::Main).await,
        Some(LiveState::Busy)
    );
}

#[tokio::test]
async fn a_rate_limit_cools_search_live_for_the_clamped_retry_after() {
    let store = Arc::new(SharedEgress::default());
    let gate = offline_gate(store.clone(), 8);

    assert_eq!(gate.cool_down(Some(30)).await, 120);
    assert_eq!(
        store.published(),
        [(BREAKER_CHANNEL.to_owned(), Duration::from_secs(120))]
    );
    assert_eq!(
        state_of(&gate, LiveClass::Main).await,
        Some(LiveState::Cooling)
    );

    assert_eq!(
        gate.open(LiveClass::Main, &identity()).await.err(),
        Some(Closed::new(LiveState::Cooling, 120)),
        "a cooling gate reports the cooldown it still has, not a fixed minute"
    );

    assert_eq!(cooling_seconds(None), 300);
    assert_eq!(cooling_seconds(Some(400)), 400);
    assert_eq!(cooling_seconds(Some(86_400)), 900);
}

#[tokio::test]
async fn a_rate_limit_shuts_this_node_before_the_shared_row_is_read_back() {
    let gate = offline_gate(Arc::new(UnreachableEgress), 8);

    gate.cool_down(Some(200)).await;

    assert_eq!(
        gate.open(LiveClass::Main, &identity()).await.err(),
        Some(Closed::new(LiveState::Cooling, 200)),
        "the next leader must not reach SoundCloud while the row is on its way"
    );
    assert!(
        !gate.proxy_admits().await,
        "a leader already past the gate must not race the proxy into another 429"
    );
    gate.record_answer().await;
    assert_eq!(
        state_of(&gate, LiveClass::Main).await,
        Some(LiveState::Cooling),
        "an answer that was already in flight does not lift a rate-limit cooldown"
    );
}

#[tokio::test(start_paused = true)]
async fn import_pacing_waits_out_the_budget_for_at_most_fifteen_seconds() {
    let checks = AtomicUsize::new(0);
    let started = tokio::time::Instant::now();

    let decision = paced(LiveClass::Import, || {
        checks.fetch_add(1, Ordering::SeqCst);
        async {
            Decision::Limited {
                retry_after_seconds: 4,
            }
        }
    })
    .await;

    assert!(matches!(decision, Decision::Limited { .. }));
    assert_eq!(started.elapsed(), Duration::from_secs(15));
    assert_eq!(checks.load(Ordering::SeqCst), 5, "at 0, 4, 8, 12 and 15 s");
}

#[tokio::test(start_paused = true)]
async fn import_pacing_goes_through_as_soon_as_the_budget_frees_up() {
    let checks = AtomicUsize::new(0);
    let started = tokio::time::Instant::now();

    let decision = paced(LiveClass::Import, || {
        let seen = checks.fetch_add(1, Ordering::SeqCst);
        async move {
            if seen < 2 {
                Decision::Limited {
                    retry_after_seconds: 3,
                }
            } else {
                Decision::Allowed
            }
        }
    })
    .await;

    assert_eq!(decision, Decision::Allowed);
    assert_eq!(started.elapsed(), Duration::from_secs(6));
}

#[tokio::test(start_paused = true)]
async fn only_the_import_is_paced_and_a_listener_is_answered_at_once() {
    let started = tokio::time::Instant::now();
    let decision = paced(LiveClass::Main, || async {
        Decision::Limited {
            retry_after_seconds: 40,
        }
    })
    .await;

    assert!(matches!(decision, Decision::Limited { .. }));
    assert_eq!(started.elapsed(), Duration::ZERO);
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn an_exhausted_budget_beats_the_in_flight_cap() {
    let spent = AdmissionLimitCfg {
        per_client: 0,
        global: 0,
    };
    let gate = LiveGate::new(
        Arc::new(SharedEgress::default()),
        admission(&redis_url(), spent),
        0,
    );

    let closed = gate
        .open(LiveClass::Main, &identity())
        .await
        .expect_err("nothing is admitted");
    assert_eq!(closed.state, LiveState::Limited);
    assert!(closed.retry_after.is_some_and(|seconds| seconds > 0));
}

#[tokio::test]
#[ignore = "requires a local Redis"]
async fn the_in_flight_cap_is_never_queued_on() {
    let generous = AdmissionLimitCfg {
        per_client: 1000,
        global: 1000,
    };
    let gate = LiveGate::new(
        Arc::new(SharedEgress::default()),
        admission(&redis_url(), generous),
        1,
    );

    let held = gate
        .open(LiveClass::Main, &identity())
        .await
        .expect("the first search goes out");
    assert_eq!(
        state_of(&gate, LiveClass::Main).await,
        Some(LiveState::Busy)
    );
    drop(held);
    assert_eq!(state_of(&gate, LiveClass::Main).await, None);
}
