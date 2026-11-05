use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::atomic::{AtomicU32, Ordering};

const MINUTE_TICKS: usize = 6;

pub struct Effort {
    floor: u32,
    cap: u32,
    budget: u64,
    used: AtomicU32,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    level: u32,
    ticks: VecDeque<u32>,
}

impl Effort {
    pub fn new(floor: u32, cap: u32, per_min: u32) -> Self {
        Self {
            floor,
            cap,
            budget: u64::from(per_min),
            used: AtomicU32::new(0),
            state: Mutex::default(),
        }
    }

    pub fn note(&self) {
        self.used.fetch_add(1, Ordering::Relaxed);
    }

    pub fn current(&self) -> u32 {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.level.max(self.floor)
    }

    pub fn minimum(&self) -> u32 {
        (self.current() / 2).max(self.floor)
    }

    pub fn tick(&self) {
        let used = self.used.swap(0, Ordering::Relaxed);
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.ticks.push_back(used);
        if state.ticks.len() > MINUTE_TICKS {
            state.ticks.pop_front();
        }
        let minute: u64 = state.ticks.iter().map(|&count| u64::from(count)).sum();
        if minute * 2 > self.budget {
            state.level = state.level.max(self.floor).saturating_mul(2).min(self.cap);
        } else if minute * 10 < self.budget {
            state.level /= 2;
        }
    }
}
