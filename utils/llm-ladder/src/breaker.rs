use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

use crate::config::BreakerSettings;

pub(crate) struct Breaker {
    failures: usize,
    window: Duration,
    open_for: Duration,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    recent: VecDeque<Instant>,
    open_until: Option<Instant>,
    probing: bool,
}

impl Breaker {
    pub fn new(settings: BreakerSettings) -> Self {
        Self {
            failures: settings.failures as usize,
            window: Duration::from_secs(settings.window_s),
            open_for: Duration::from_secs(settings.open_s),
            state: Mutex::default(),
        }
    }

    pub fn is_open(&self, now: Instant) -> bool {
        let state = self.state();
        state
            .open_until
            .is_some_and(|until| now < until || state.probing)
    }

    pub fn enter(&self, now: Instant) -> bool {
        let mut state = self.state();
        match state.open_until {
            None => true,
            Some(until) if now < until || state.probing => false,
            Some(_) => {
                state.probing = true;
                true
            }
        }
    }

    pub fn succeeded(&self) {
        *self.state() = State::default();
    }

    pub fn failed(&self, now: Instant) {
        let mut state = self.state();
        if state.open_until.is_some() {
            state.probing = false;
            state.open_until = Some(now + self.open_for);
            return;
        }
        state.recent.push_back(now);
        while state
            .recent
            .front()
            .is_some_and(|at| now.duration_since(*at) > self.window)
        {
            state.recent.pop_front();
        }
        if state.recent.len() >= self.failures {
            state.recent.clear();
            state.open_until = Some(now + self.open_for);
        }
    }

    pub fn left(&self) {
        self.state().probing = false;
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn breaker() -> Breaker {
        Breaker::new(BreakerSettings {
            failures: 3,
            window_s: 60,
            open_s: 120,
        })
    }

    #[tokio::test(start_paused = true)]
    async fn failures_inside_the_window_open_it_for_a_while() {
        let breaker = breaker();
        for _ in 0..2 {
            assert!(breaker.enter(Instant::now()));
            breaker.failed(Instant::now());
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        assert!(!breaker.is_open(Instant::now()));
        breaker.failed(Instant::now());
        assert!(breaker.is_open(Instant::now()));
        assert!(!breaker.enter(Instant::now()));
        tokio::time::advance(Duration::from_secs(119)).await;
        assert!(breaker.is_open(Instant::now()));
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(!breaker.is_open(Instant::now()));
    }

    #[tokio::test(start_paused = true)]
    async fn failures_spread_wider_than_the_window_do_not_open_it() {
        let breaker = breaker();
        for _ in 0..6 {
            breaker.failed(Instant::now());
            tokio::time::advance(Duration::from_secs(40)).await;
        }
        assert!(!breaker.is_open(Instant::now()));
    }

    #[tokio::test(start_paused = true)]
    async fn after_the_pause_one_call_probes_and_its_result_decides() {
        let breaker = breaker();
        for _ in 0..3 {
            breaker.failed(Instant::now());
        }
        tokio::time::advance(Duration::from_secs(121)).await;
        assert!(breaker.enter(Instant::now()), "the probe goes through");
        assert!(!breaker.enter(Instant::now()), "only one probe at a time");
        assert!(breaker.is_open(Instant::now()));
        breaker.failed(Instant::now());
        assert!(!breaker.enter(Instant::now()), "a failed probe reopens it");
        tokio::time::advance(Duration::from_secs(121)).await;
        assert!(breaker.enter(Instant::now()));
        breaker.succeeded();
        assert!(breaker.enter(Instant::now()));
        assert!(!breaker.is_open(Instant::now()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_probe_that_was_cancelled_frees_the_slot() {
        let breaker = breaker();
        for _ in 0..3 {
            breaker.failed(Instant::now());
        }
        tokio::time::advance(Duration::from_secs(121)).await;
        assert!(breaker.enter(Instant::now()));
        breaker.left();
        assert!(breaker.enter(Instant::now()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_success_forgets_earlier_failures() {
        let breaker = breaker();
        breaker.failed(Instant::now());
        breaker.failed(Instant::now());
        breaker.succeeded();
        breaker.failed(Instant::now());
        breaker.failed(Instant::now());
        assert!(!breaker.is_open(Instant::now()));
    }
}
