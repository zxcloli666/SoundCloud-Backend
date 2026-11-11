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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pass {
    Regular,
    Probe,
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

    pub fn enter(&self, now: Instant) -> Option<Pass> {
        let mut state = self.state();
        match state.open_until {
            None => Some(Pass::Regular),
            Some(until) if now < until || state.probing => None,
            Some(_) => {
                state.probing = true;
                Some(Pass::Probe)
            }
        }
    }

    pub fn succeeded(&self, pass: Pass) {
        let mut state = self.state();
        if pass == Pass::Probe || state.open_until.is_none() {
            *state = State::default();
        }
    }

    pub fn failed(&self, pass: Pass, now: Instant) {
        let mut state = self.state();
        match (pass, state.open_until) {
            (Pass::Probe, _) => {
                state.probing = false;
                state.open_until = Some(now + self.open_for);
            }
            (Pass::Regular, Some(_)) => {}
            (Pass::Regular, None) => self.count(&mut state, now),
        }
    }

    pub fn left(&self, pass: Pass) {
        if pass == Pass::Probe {
            self.state().probing = false;
        }
    }

    fn count(&self, state: &mut State, now: Instant) {
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
            assert!(breaker.enter(Instant::now()).is_some());
            breaker.failed(Pass::Regular, Instant::now());
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        assert!(!breaker.is_open(Instant::now()));
        breaker.failed(Pass::Regular, Instant::now());
        assert!(breaker.is_open(Instant::now()));
        assert!(breaker.enter(Instant::now()).is_none());
        tokio::time::advance(Duration::from_secs(119)).await;
        assert!(breaker.is_open(Instant::now()));
        tokio::time::advance(Duration::from_secs(2)).await;
        assert!(!breaker.is_open(Instant::now()));
    }

    #[tokio::test(start_paused = true)]
    async fn failures_spread_wider_than_the_window_do_not_open_it() {
        let breaker = breaker();
        for _ in 0..6 {
            breaker.failed(Pass::Regular, Instant::now());
            tokio::time::advance(Duration::from_secs(40)).await;
        }
        assert!(!breaker.is_open(Instant::now()));
    }

    #[tokio::test(start_paused = true)]
    async fn after_the_pause_one_call_probes_and_its_result_decides() {
        let breaker = breaker();
        for _ in 0..3 {
            breaker.failed(Pass::Regular, Instant::now());
        }
        tokio::time::advance(Duration::from_secs(121)).await;
        assert_eq!(breaker.enter(Instant::now()), Some(Pass::Probe));
        assert_eq!(breaker.enter(Instant::now()), None, "one probe at a time");
        assert!(breaker.is_open(Instant::now()));
        breaker.failed(Pass::Probe, Instant::now());
        assert_eq!(
            breaker.enter(Instant::now()),
            None,
            "a failed probe reopens it"
        );
        tokio::time::advance(Duration::from_secs(121)).await;
        assert_eq!(breaker.enter(Instant::now()), Some(Pass::Probe));
        breaker.succeeded(Pass::Probe);
        assert_eq!(breaker.enter(Instant::now()), Some(Pass::Regular));
        assert!(!breaker.is_open(Instant::now()));
    }

    #[tokio::test(start_paused = true)]
    async fn a_probe_that_was_cancelled_frees_the_slot() {
        let breaker = breaker();
        for _ in 0..3 {
            breaker.failed(Pass::Regular, Instant::now());
        }
        tokio::time::advance(Duration::from_secs(121)).await;
        assert_eq!(breaker.enter(Instant::now()), Some(Pass::Probe));
        breaker.left(Pass::Probe);
        assert_eq!(breaker.enter(Instant::now()), Some(Pass::Probe));
    }

    #[tokio::test(start_paused = true)]
    async fn only_its_probe_changes_an_open_breaker() {
        let breaker = breaker();
        assert_eq!(breaker.enter(Instant::now()), Some(Pass::Regular));
        for _ in 0..3 {
            breaker.failed(Pass::Regular, Instant::now());
        }
        tokio::time::advance(Duration::from_secs(121)).await;
        assert_eq!(breaker.enter(Instant::now()), Some(Pass::Probe));

        breaker.left(Pass::Regular);
        assert_eq!(
            breaker.enter(Instant::now()),
            None,
            "the slot is the probe's"
        );
        breaker.failed(Pass::Regular, Instant::now());
        assert_eq!(breaker.enter(Instant::now()), None);
        breaker.succeeded(Pass::Regular);
        assert!(
            breaker.is_open(Instant::now()),
            "a late answer closes nothing"
        );

        breaker.failed(Pass::Probe, Instant::now());
        tokio::time::advance(Duration::from_secs(119)).await;
        assert!(
            breaker.is_open(Instant::now()),
            "the pause runs from the probe"
        );
        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(breaker.enter(Instant::now()), Some(Pass::Probe));
    }

    #[tokio::test(start_paused = true)]
    async fn a_success_forgets_earlier_failures() {
        let breaker = breaker();
        breaker.failed(Pass::Regular, Instant::now());
        breaker.failed(Pass::Regular, Instant::now());
        breaker.succeeded(Pass::Regular);
        breaker.failed(Pass::Regular, Instant::now());
        breaker.failed(Pass::Regular, Instant::now());
        assert!(!breaker.is_open(Instant::now()));
    }
}
