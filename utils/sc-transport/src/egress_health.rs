use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use tokio::sync::Mutex;
use tracing::debug;

use crate::RelayRead;
use crate::channel_health::{COOLDOWN, ChannelHealth, Trip, now_ms};

pub const EGRESS_RELAY_LUA: &str = "relay_lua";
pub const EGRESS_RELAY_RAW: &str = "relay_raw";

const SHARED_REFRESH: Duration = Duration::from_secs(1);
const STORE_BUDGET: Duration = Duration::from_millis(500);

pub type EgressFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EgressState {
    Closed,
    Open(Duration),
    Unknown,
}

pub trait EgressHealthStore: Send + Sync {
    fn publish_open<'a>(
        &'a self,
        channel: &'a str,
        app: &'a str,
        cooldown: Duration,
    ) -> EgressFuture<'a, ()>;

    fn publish_closed<'a>(&'a self, channel: &'a str) -> EgressFuture<'a, ()>;

    fn remaining<'a>(&'a self, channel: &'a str) -> EgressFuture<'a, EgressState>;
}

pub struct EgressHealth {
    channel: &'static str,
    app: &'static str,
    local: ChannelHealth,
    store: Option<Arc<dyn EgressHealthStore>>,
    shared_open_until_ms: AtomicI64,
    refreshed_at_ms: AtomicI64,
    refresh: Mutex<()>,
}

impl EgressHealth {
    pub fn new(
        channel: &'static str,
        app: &'static str,
        store: Option<Arc<dyn EgressHealthStore>>,
    ) -> Self {
        Self {
            channel,
            app,
            local: ChannelHealth::default(),
            store,
            shared_open_until_ms: AtomicI64::new(0),
            refreshed_at_ms: AtomicI64::new(0),
            refresh: Mutex::new(()),
        }
    }

    pub async fn is_open(&self) -> bool {
        if self.local.is_open() {
            return true;
        }
        self.refresh_shared().await;
        self.shared_is_open()
    }

    pub async fn observe<T>(&self, read: &RelayRead<T>) -> bool {
        self.settle(self.local.observe(read)).await
    }

    pub async fn record_ok(&self) -> bool {
        self.settle(self.local.record_ok()).await
    }

    pub async fn record_ban(&self) -> bool {
        self.settle(self.local.record_ban()).await
    }

    pub async fn record_answer(&self, answered: bool) -> bool {
        if answered {
            self.record_ok().await
        } else {
            self.record_ban().await
        }
    }

    async fn settle(&self, trip: Trip) -> bool {
        if let Some(store) = self.store.as_ref() {
            match trip {
                Trip::Steady => {}
                Trip::Opened => {
                    let write = store.publish_open(self.channel, self.app, COOLDOWN);
                    if tokio::time::timeout(STORE_BUDGET, write).await.is_err() {
                        debug!(
                            channel = self.channel,
                            "publishing the open breaker timed out"
                        );
                    }
                }
                Trip::Closed => {
                    let write = store.publish_closed(self.channel);
                    if tokio::time::timeout(STORE_BUDGET, write).await.is_err() {
                        debug!(
                            channel = self.channel,
                            "publishing the closed breaker timed out"
                        );
                    }
                    self.shared_open_until_ms.store(0, Ordering::Release);
                    self.refreshed_at_ms.store(now_ms(), Ordering::Release);
                }
            }
        }
        self.local.is_open() || self.shared_is_open()
    }

    fn shared_is_open(&self) -> bool {
        now_ms() < self.shared_open_until_ms.load(Ordering::Acquire)
    }

    fn fresh_enough(&self) -> bool {
        now_ms() - self.refreshed_at_ms.load(Ordering::Acquire) < SHARED_REFRESH.as_millis() as i64
    }

    async fn refresh_shared(&self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        if self.fresh_enough() {
            return;
        }
        let Ok(_guard) = self.refresh.try_lock() else {
            return;
        };
        if self.fresh_enough() {
            return;
        }
        self.refreshed_at_ms.store(now_ms(), Ordering::Release);
        let state = match tokio::time::timeout(STORE_BUDGET, store.remaining(self.channel)).await {
            Ok(state) => state,
            Err(_) => EgressState::Unknown,
        };
        match state {
            EgressState::Open(left) => self
                .shared_open_until_ms
                .store(now_ms() + left.as_millis() as i64, Ordering::Release),
            EgressState::Closed => self.shared_open_until_ms.store(0, Ordering::Release),
            EgressState::Unknown => debug!(
                channel = self.channel,
                "shared egress health is unreadable, keeping the last known state"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    struct FakeStore {
        state: StdMutex<EgressState>,
        reads: AtomicUsize,
        opens: AtomicUsize,
        closes: AtomicUsize,
        silent: AtomicBool,
    }

    impl FakeStore {
        fn holding(state: EgressState) -> Arc<Self> {
            Arc::new(Self {
                state: StdMutex::new(state),
                reads: AtomicUsize::new(0),
                opens: AtomicUsize::new(0),
                closes: AtomicUsize::new(0),
                silent: AtomicBool::new(false),
            })
        }

        fn open_for(seconds: u64) -> Arc<Self> {
            Self::holding(EgressState::Open(Duration::from_secs(seconds)))
        }

        fn closed() -> Arc<Self> {
            Self::holding(EgressState::Closed)
        }
    }

    impl EgressHealthStore for FakeStore {
        fn publish_open<'a>(
            &'a self,
            _channel: &'a str,
            _app: &'a str,
            cooldown: Duration,
        ) -> EgressFuture<'a, ()> {
            Box::pin(async move {
                self.opens.fetch_add(1, Ordering::SeqCst);
                *self.state.lock().unwrap() = EgressState::Open(cooldown);
            })
        }

        fn publish_closed<'a>(&'a self, _channel: &'a str) -> EgressFuture<'a, ()> {
            Box::pin(async move {
                self.closes.fetch_add(1, Ordering::SeqCst);
                *self.state.lock().unwrap() = EgressState::Closed;
            })
        }

        fn remaining<'a>(&'a self, _channel: &'a str) -> EgressFuture<'a, EgressState> {
            Box::pin(async move {
                self.reads.fetch_add(1, Ordering::SeqCst);
                if self.silent.load(Ordering::SeqCst) {
                    std::future::pending::<()>().await;
                }
                *self.state.lock().unwrap()
            })
        }
    }

    fn health(store: Option<Arc<dyn EgressHealthStore>>) -> EgressHealth {
        EgressHealth::new(EGRESS_RELAY_LUA, "test", store)
    }

    #[tokio::test]
    async fn a_channel_another_process_opened_blocks_us_without_one_local_failure() {
        let store = FakeStore::open_for(60);
        let health = health(Some(store.clone()));

        assert!(
            health.is_open().await,
            "a breaker opened by another process must block this one too"
        );
        assert_eq!(store.opens.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn an_outage_is_published_once_and_the_recovery_publishes_the_close() {
        let store = FakeStore::closed();
        let health = health(Some(store.clone()));

        for _ in 0..24 {
            health.observe(&RelayRead::<()>::Unavailable).await;
        }
        assert_eq!(
            store.opens.load(Ordering::SeqCst),
            1,
            "an outage must reach the shared state once, not once per failed call"
        );

        assert!(!health.observe(&RelayRead::<()>::Found(())).await);
        assert_eq!(store.closes.load(Ordering::SeqCst), 1);
        for _ in 0..8 {
            health.observe(&RelayRead::<()>::Found(())).await;
        }
        assert_eq!(store.closes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_local_trip_wins_even_when_the_shared_state_says_the_channel_is_fine() {
        let store = FakeStore::closed();
        let health = health(Some(store.clone()));
        health.refresh_shared().await;
        *store.state.lock().unwrap() = EgressState::Closed;
        store.opens.store(0, Ordering::SeqCst);

        for _ in 0..4 {
            health.local.record_ban();
        }
        assert!(
            health.is_open().await,
            "our own observation must not be overridden by a healthy shared state"
        );
    }

    #[tokio::test]
    async fn the_shared_state_is_read_at_most_once_a_second() {
        let store = FakeStore::closed();
        let health = health(Some(store.clone()));

        for _ in 0..64 {
            assert!(!health.is_open().await);
        }
        assert_eq!(
            store.reads.load(Ordering::SeqCst),
            1,
            "the hot path must not pay a read per call"
        );

        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert!(!health.is_open().await);
        assert_eq!(
            store.reads.load(Ordering::SeqCst),
            2,
            "after the window the shared state must be read again"
        );
    }

    #[tokio::test]
    async fn a_silent_store_costs_one_budget_and_keeps_the_last_known_state() {
        let store = FakeStore::open_for(60);
        let health = health(Some(store.clone()));
        assert!(health.is_open().await);

        store.silent.store(true, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(1100)).await;

        let started = std::time::Instant::now();
        assert!(
            health.is_open().await,
            "an unreadable store must not close a breaker that was open"
        );
        assert!(
            started.elapsed() < STORE_BUDGET * 2,
            "a silent store must cost one budget, not the whole call"
        );

        let before = store.reads.load(Ordering::SeqCst);
        for _ in 0..32 {
            health.is_open().await;
        }
        assert_eq!(
            store.reads.load(Ordering::SeqCst),
            before,
            "a sick store must be asked at most once a second, not on every call"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_wave_of_callers_reads_the_shared_state_once() {
        let store = FakeStore::closed();
        let health = Arc::new(health(Some(store.clone())));

        let callers: Vec<_> = (0..32)
            .map(|_| {
                let health = health.clone();
                tokio::spawn(async move { health.is_open().await })
            })
            .collect();
        for caller in callers {
            assert!(!caller.await.unwrap());
        }

        assert_eq!(
            store.reads.load(Ordering::SeqCst),
            1,
            "a burst must single-flight the shared read"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn an_outage_seen_by_many_callers_at_once_stops_the_next_wave() {
        let store = FakeStore::closed();
        let health = Arc::new(health(Some(store.clone())));
        let attempts = Arc::new(AtomicUsize::new(0));

        async fn wave(
            health: Arc<EgressHealth>,
            attempts: Arc<AtomicUsize>,
            callers: usize,
        ) -> usize {
            let started = attempts.load(Ordering::SeqCst);
            let callers: Vec<_> = (0..callers)
                .map(|_| {
                    let health = health.clone();
                    let attempts = attempts.clone();
                    tokio::spawn(async move {
                        if health.is_open().await {
                            return;
                        }
                        attempts.fetch_add(1, Ordering::SeqCst);
                        tokio::task::yield_now().await;
                        health.observe(&RelayRead::<()>::Unavailable).await;
                    })
                })
                .collect();
            for caller in callers {
                caller.await.expect("caller finishes");
            }
            attempts.load(Ordering::SeqCst) - started
        }

        let first = wave(health.clone(), attempts.clone(), 32).await;
        assert!(first > 0, "the first wave must actually reach the relay");
        assert!(
            health.is_open().await,
            "a relay that answers nobody must end the wave with an open breaker"
        );
        assert_eq!(
            store.opens.load(Ordering::SeqCst),
            1,
            "a fan-out outage must reach the shared state once"
        );

        let second = wave(health.clone(), attempts.clone(), 32).await;
        assert_eq!(
            second, 0,
            "once the breaker is open every caller must take the backup instead of the relay"
        );
        assert_eq!(
            store.reads.load(Ordering::SeqCst),
            1,
            "an open local breaker must answer without touching the shared state"
        );
    }

    #[tokio::test]
    async fn without_a_store_the_breaker_stays_purely_local() {
        let health = health(None);
        for _ in 0..8 {
            health.observe(&RelayRead::<()>::Unavailable).await;
        }
        assert!(health.is_open().await);
        assert!(!health.observe(&RelayRead::<()>::Found(())).await);
        assert!(!health.is_open().await);
    }

    #[tokio::test]
    async fn a_shared_open_expires_on_its_own_when_nobody_republishes_it() {
        let store = FakeStore::holding(EgressState::Open(Duration::from_millis(200)));
        let health = health(Some(store.clone()));
        assert!(health.is_open().await);

        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            !health.is_open().await,
            "a shared open must lapse with its own deadline, not hold forever"
        );
    }
}
