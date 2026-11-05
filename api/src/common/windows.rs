use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

pub const AUTH_WINDOW: Duration = Duration::from_secs(60);
pub const CLIENTS_TRACKED: usize = 10_000;

pub struct Window {
    limit: u32,
    opened: Mutex<(Instant, u32)>,
}

impl Window {
    pub fn new(limit: u32) -> Self {
        Self {
            limit,
            opened: Mutex::new((Instant::now(), 0)),
        }
    }

    pub fn admit(&self, now: Instant) -> Result<(), u64> {
        let mut opened = self.opened.lock().unwrap_or_else(PoisonError::into_inner);
        let (started, used) = &mut *opened;
        if now.saturating_duration_since(*started) >= AUTH_WINDOW {
            *started = now;
            *used = 0;
        }
        if *used >= self.limit {
            return Err(retry_ms(AUTH_WINDOW, *started, now));
        }
        *used += 1;
        Ok(())
    }

    pub fn full(&self, now: Instant) -> bool {
        let opened = self.opened.lock().unwrap_or_else(PoisonError::into_inner);
        let (started, used) = *opened;
        now.saturating_duration_since(started) < AUTH_WINDOW && used >= self.limit
    }
}

pub struct Clients {
    limit: u32,
    window: Duration,
    state: Mutex<Tracked>,
}

#[derive(Default)]
struct Tracked {
    windows: HashMap<String, (Instant, u32)>,
    opened: VecDeque<(Instant, String)>,
}

impl Clients {
    pub fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit,
            window,
            state: Mutex::new(Tracked::default()),
        }
    }

    pub fn admit(&self, client: &str, now: Instant) -> Result<(), u64> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let tracked = &mut *state;
        if let Some((started, used)) = tracked.windows.get_mut(client) {
            if now.saturating_duration_since(*started) < self.window {
                if *used >= self.limit {
                    return Err(retry_ms(self.window, *started, now));
                }
                *used += 1;
                return Ok(());
            }
            *started = now;
            *used = 1;
            tracked.opened.push_back((now, client.to_owned()));
            tracked.compact();
            return Ok(());
        }
        if tracked.windows.len() >= CLIENTS_TRACKED {
            tracked.evict_oldest();
        }
        tracked.windows.insert(client.to_owned(), (now, 1));
        tracked.opened.push_back((now, client.to_owned()));
        tracked.compact();
        Ok(())
    }

    #[cfg(test)]
    pub fn tracked(&self) -> (usize, usize) {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        (state.windows.len(), state.opened.len())
    }
}

impl Tracked {
    fn evict_oldest(&mut self) {
        while let Some((started, client)) = self.opened.pop_front() {
            if self
                .windows
                .get(&client)
                .is_some_and(|(current, _)| *current == started)
            {
                self.windows.remove(&client);
                return;
            }
        }
    }

    fn compact(&mut self) {
        if self.opened.len() <= 2 * CLIENTS_TRACKED {
            return;
        }
        let mut live: Vec<(Instant, String)> = self
            .windows
            .iter()
            .map(|(client, (started, _))| (*started, client.clone()))
            .collect();
        live.sort();
        self.opened = live.into();
    }
}

pub fn retry_ms(window: Duration, started: Instant, now: Instant) -> u64 {
    let left = window.saturating_sub(now.saturating_duration_since(started));
    u64::try_from(left.as_millis()).unwrap_or(u64::MAX).max(1)
}
