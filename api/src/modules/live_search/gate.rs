use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use sc_transport::{EgressHealth, EgressHealthStore};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::warn;

use super::meta::LiveState;
use super::query::{IMPORT_PACING, LiveClass};
use crate::common::admission::{Decision, Endpoint, PublicAdmission};
use crate::sc::EGRESS_APP;

pub const PAUSE_CHANNEL: &str = "search_live_pause";
pub const BREAKER_CHANNEL: &str = "search_live";

const BUSY_RETRY_AFTER: i64 = 1;
const COOLING_DEFAULT: i64 = 300;
const COOLING_MIN: i64 = 120;
const COOLING_MAX: i64 = 900;
const PUBLISH_BUDGET: Duration = Duration::from_millis(500);
const WARNING_INTERVAL_MS: i64 = 30_000;
const PROXY_IDENTITY: &str = "node";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Closed {
    pub state: LiveState,
    pub retry_after: Option<i64>,
}

impl Closed {
    pub fn new(state: LiveState, retry_after: i64) -> Self {
        Self {
            state,
            retry_after: Some(retry_after),
        }
    }
}

pub struct LiveGate {
    pause: EgressHealth,
    breaker: EgressHealth,
    store: Arc<dyn EgressHealthStore>,
    admission: Arc<PublicAdmission>,
    in_flight: Arc<Semaphore>,
    cooling_until_ms: AtomicI64,
    breaker_open: AtomicBool,
    warned_at_ms: AtomicI64,
}

impl LiveGate {
    pub fn new(
        store: Arc<dyn EgressHealthStore>,
        admission: Arc<PublicAdmission>,
        max_in_flight: usize,
    ) -> Self {
        Self {
            pause: EgressHealth::new(PAUSE_CHANNEL, EGRESS_APP, Some(store.clone())),
            breaker: EgressHealth::new(BREAKER_CHANNEL, EGRESS_APP, Some(store.clone())),
            store,
            admission,
            in_flight: Arc::new(Semaphore::new(max_in_flight)),
            cooling_until_ms: AtomicI64::new(0),
            breaker_open: AtomicBool::new(false),
            warned_at_ms: AtomicI64::new(0),
        }
    }

    pub async fn open(
        &self,
        class: LiveClass,
        identity: &str,
    ) -> Result<OwnedSemaphorePermit, Closed> {
        self.shut().await?;
        let decision = paced(class, || {
            self.admission.check_identity(class.endpoint(), identity)
        })
        .await;
        self.shut().await?;
        match decision {
            Decision::Allowed => {}
            Decision::Limited {
                retry_after_seconds,
            } => {
                return Err(Closed::new(
                    LiveState::Limited,
                    i64::try_from(retry_after_seconds).unwrap_or(i64::MAX),
                ));
            }
            Decision::Unavailable => return Err(Closed::new(LiveState::Busy, BUSY_RETRY_AFTER)),
        }
        self.in_flight
            .clone()
            .try_acquire_owned()
            .map_err(|_| Closed::new(LiveState::Busy, BUSY_RETRY_AFTER))
    }

    pub async fn proxy_admits(&self) -> bool {
        self.cooling_left().is_none()
            && self
                .admission
                .check_identity(Endpoint::LiveProxy, PROXY_IDENTITY)
                .await
                == Decision::Allowed
    }

    pub async fn record_answer(&self) {
        let open = self.breaker.record_ok().await;
        self.note_breaker(open || self.cooling_left().is_some());
    }

    pub async fn record_silence(&self) {
        let open = self.breaker.record_ban().await;
        self.note_breaker(open || self.cooling_left().is_some());
    }

    pub async fn cool_down(&self, retry_after: Option<i64>) -> i64 {
        let seconds = cooling_seconds(retry_after);
        self.cooling_until_ms
            .fetch_max(now_ms() + seconds * 1000, Ordering::AcqRel);
        self.note_breaker(true);
        if self.warning_due() {
            warn!(
                retry_after,
                cooldown_seconds = seconds,
                "SoundCloud rate-limited live search, cooling search_live"
            );
        }
        let publish = self.store.publish_open(
            BREAKER_CHANNEL,
            EGRESS_APP,
            Duration::from_secs(seconds.unsigned_abs()),
        );
        let _ = tokio::time::timeout(PUBLISH_BUDGET, publish).await;
        seconds
    }

    async fn shut(&self) -> Result<(), Closed> {
        let paused = self.pause.open_for().await;
        crate::metrics::set_live_gate_closed("pause", paused.is_some());
        if let Some(left) = paused {
            return Err(Closed::new(LiveState::Paused, whole_seconds(left)));
        }
        let cooling = self.breaker.open_for().await.max(self.cooling_left());
        crate::metrics::set_live_gate_closed("breaker", cooling.is_some());
        self.note_breaker(cooling.is_some());
        if let Some(left) = cooling {
            return Err(Closed::new(LiveState::Cooling, whole_seconds(left)));
        }
        Ok(())
    }

    fn cooling_left(&self) -> Option<Duration> {
        let left = self.cooling_until_ms.load(Ordering::Acquire) - now_ms();
        (left > 0).then(|| Duration::from_millis(left.unsigned_abs()))
    }

    fn note_breaker(&self, open: bool) {
        if self.breaker_open.swap(open, Ordering::AcqRel) == open {
            return;
        }
        crate::metrics::set_live_gate_closed("breaker", open);
        if open {
            warn!(channel = BREAKER_CHANNEL, "live search breaker opened");
        } else {
            warn!(channel = BREAKER_CHANNEL, "live search breaker closed");
        }
    }

    fn warning_due(&self) -> bool {
        let now = now_ms();
        let previous = self.warned_at_ms.load(Ordering::Relaxed);
        now - previous >= WARNING_INTERVAL_MS
            && self
                .warned_at_ms
                .compare_exchange(previous, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn whole_seconds(left: Duration) -> i64 {
    i64::try_from(left.as_millis().div_ceil(1000))
        .unwrap_or(i64::MAX)
        .max(1)
}

pub fn cooling_seconds(retry_after: Option<i64>) -> i64 {
    retry_after
        .unwrap_or(COOLING_DEFAULT)
        .clamp(COOLING_MIN, COOLING_MAX)
}

pub async fn paced<F, Fut>(class: LiveClass, mut check: F) -> Decision
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Decision>,
{
    let started = tokio::time::Instant::now();
    loop {
        let decision = check().await;
        let Decision::Limited {
            retry_after_seconds,
        } = decision
        else {
            return decision;
        };
        let left = IMPORT_PACING.saturating_sub(started.elapsed());
        if class != LiveClass::Import || left.is_zero() {
            return decision;
        }
        tokio::time::sleep(Duration::from_secs(retry_after_seconds.max(1)).min(left)).await;
    }
}
