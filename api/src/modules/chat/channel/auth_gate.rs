use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request};
use ipnet::IpNet;

use crate::common::admission::{client_address, client_identity};
use crate::modules::chat::config::StartLimits;
use crate::modules::chat::errors::BusySource;
use crate::modules::chat::{ChatError, ChatResult};

pub const AUTH_WINDOW: Duration = Duration::from_secs(60);
pub const LOGINS_PER_WINDOW: u32 = 3_000;
pub const LINKS_PER_WINDOW: u32 = 3_000;
pub const CLIENTS_TRACKED: usize = 10_000;

pub struct AuthGate {
    trusted: Vec<IpNet>,
    logins: Window,
    links: Window,
    login_clients: Clients,
    link_clients: Clients,
}

impl AuthGate {
    pub fn new(limits: &StartLimits) -> Self {
        Self {
            trusted: limits.trusted_proxies.clone(),
            logins: Window::new(LOGINS_PER_WINDOW),
            links: Window::new(LINKS_PER_WINDOW),
            login_clients: Clients::new(limits.logins_per_client, limits.window),
            link_clients: Clients::new(limits.links_per_client, limits.window),
        }
    }

    pub fn client(&self, request: &Request) -> Option<IpAddr> {
        let peer = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()?
            .0
            .ip();
        Some(client_address(peer, request.headers(), &self.trusted))
    }

    pub fn admit(&self, path: &str, client: Option<IpAddr>) -> ChatResult<()> {
        let (window, clients) = match path {
            "/auth/login" => (&self.logins, &self.login_clients),
            "/auth/link/create" => (&self.links, &self.link_clients),
            _ => return Ok(()),
        };
        let client = client.ok_or(ChatError::Unavailable)?;
        let now = Instant::now();
        clients
            .admit(&client_identity(client), now)
            .and_then(|()| window.admit(now))
            .map_err(|retry_ms| ChatError::Busy {
                retry_ms,
                source: BusySource::AuthGate,
            })
    }

    #[cfg(test)]
    pub fn tracked(&self, path: &str) -> (usize, usize) {
        match path {
            "/auth/login" => self.login_clients.tracked(),
            "/auth/link/create" => self.link_clients.tracked(),
            _ => (0, 0),
        }
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
            return Err(retry_ms(AUTH_WINDOW, *started, now));
        }
        *used += 1;
        Ok(())
    }
}

struct Clients {
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
    fn new(limit: u32, window: Duration) -> Self {
        Self {
            limit,
            window,
            state: Mutex::new(Tracked::default()),
        }
    }

    fn admit(&self, client: &str, now: Instant) -> Result<(), u64> {
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
    fn tracked(&self) -> (usize, usize) {
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

fn retry_ms(window: Duration, started: Instant, now: Instant) -> u64 {
    let left = window.saturating_sub(now.saturating_duration_since(started));
    u64::try_from(left.as_millis()).unwrap_or(u64::MAX).max(1)
}
