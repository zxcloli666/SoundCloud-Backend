use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::modules::chat::{ChatError, ChatResult};

pub const AUTH_WINDOW: Duration = Duration::from_secs(60);
pub const LOGINS_PER_WINDOW: u32 = 300;
pub const LINKS_PER_WINDOW: u32 = 300;

pub struct AuthGate {
    logins: Window,
    links: Window,
}

impl AuthGate {
    pub fn new() -> Self {
        Self {
            logins: Window::new(LOGINS_PER_WINDOW),
            links: Window::new(LINKS_PER_WINDOW),
        }
    }

    pub fn admit(&self, path: &str) -> ChatResult<()> {
        let window = match path {
            "/auth/login" => &self.logins,
            "/auth/link/create" => &self.links,
            _ => return Ok(()),
        };
        window
            .admit(Instant::now())
            .map_err(|retry_ms| ChatError::Busy { retry_ms })
    }
}

struct Window {
    limit: u32,
    opened: Mutex<(Instant, u32)>,
}

impl Window {
    fn new(limit: u32) -> Self {
        Self {
            limit,
            opened: Mutex::new((Instant::now(), 0)),
        }
    }

    fn admit(&self, now: Instant) -> Result<(), u64> {
        let mut opened = self.opened.lock().unwrap_or_else(PoisonError::into_inner);
        let (started, used) = &mut *opened;
        if now.saturating_duration_since(*started) >= AUTH_WINDOW {
            *started = now;
            *used = 0;
        }
        if *used >= self.limit {
            let left = AUTH_WINDOW.saturating_sub(now.saturating_duration_since(*started));
            return Err(u64::try_from(left.as_millis()).unwrap_or(u64::MAX).max(1));
        }
        *used += 1;
        Ok(())
    }
}
