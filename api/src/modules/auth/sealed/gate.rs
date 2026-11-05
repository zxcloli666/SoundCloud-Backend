use std::net::IpAddr;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::common::admission::{client_identity, subnet_identity};
use crate::common::windows::{AUTH_WINDOW, Clients, Window};
use crate::modules::auth::sealed::bucket::Bucket;
use crate::modules::auth::sealed::config::SealedCfg;
use crate::modules::auth::sealed::effort::Effort;

pub const FREE_LOGINS_PER_CLIENT: u32 = 15;
pub const FREE_LINKS_PER_CLIENT: u32 = 30;
pub const POW_PER_CLIENT: u32 = 60;
pub const POW_BURST: u32 = 50;
pub const NETWORK_V4_PER_MIN: u32 = 120;
pub const NETWORK_V6_PER_MIN: u32 = 60;
pub const LOGIN_POLLS_PER_CLIENT: u32 = 120;
pub const LINK_POLLS_PER_CLIENT: u32 = 120;
pub const LINK_CLAIMS_PER_CLIENT: u32 = 30;
pub const TICK: Duration = Duration::from_secs(10);
pub const TICKS_CAUGHT_UP: u32 = 6;

pub enum Refused {
    Lane(u64),
    Capped(u64),
}

pub struct StartGate {
    path: &'static str,
    free: Window,
    free_clients: Clients,
    pow: Bucket,
    pow_clients: Clients,
    v4: Clients,
    v6: Clients,
    pub effort: Effort,
    pub ticked: Mutex<Instant>,
}

impl StartGate {
    pub fn new(path: &'static str, cfg: &SealedCfg, free_per_client: u32) -> Self {
        Self {
            path,
            free: Window::new(cfg.free_per_min),
            free_clients: Clients::new(free_per_client, AUTH_WINDOW),
            pow: Bucket::new(cfg.pow_per_min, POW_BURST),
            pow_clients: Clients::new(POW_PER_CLIENT, AUTH_WINDOW),
            v4: Clients::new(NETWORK_V4_PER_MIN, AUTH_WINDOW),
            v6: Clients::new(NETWORK_V6_PER_MIN, AUTH_WINDOW),
            effort: Effort::new(cfg.pow_floor, cfg.pow_cap, cfg.pow_per_min),
            ticked: Mutex::new(Instant::now()),
        }
    }

    pub fn admit_free(&self, client: IpAddr, now: Instant) -> Result<(), Refused> {
        self.free_clients
            .admit(&client_identity(client), now)
            .map_err(Refused::Lane)?;
        self.free.admit(now).map_err(Refused::Lane)?;
        self.admit_network(client, now)
    }

    pub fn admit_pow(&self, client: IpAddr, now: Instant) -> Result<(), Refused> {
        self.pow_clients
            .admit(&client_identity(client), now)
            .map_err(Refused::Capped)?;
        self.pow.take(now).map_err(Refused::Lane)?;
        self.effort.note();
        self.admit_network(client, now)
    }

    pub fn catch_up(&self, now: Instant) {
        let mut ticked = self.ticked.lock().unwrap_or_else(PoisonError::into_inner);
        let mut caught_up = 0;
        while now.saturating_duration_since(*ticked) >= TICK && caught_up < TICKS_CAUGHT_UP {
            *ticked += TICK;
            caught_up += 1;
            self.tick(now);
        }
        if now.saturating_duration_since(*ticked) >= TICK {
            *ticked = now;
        }
    }

    fn tick(&self, now: Instant) {
        self.effort.tick();
        metrics::gauge!("auth_pow_effort", "path" => self.path).set(f64::from(self.required(now)));
    }

    pub fn required(&self, now: Instant) -> u32 {
        match self.free.full(now) {
            true => self.effort.current(),
            false => 0,
        }
    }

    fn admit_network(&self, client: IpAddr, now: Instant) -> Result<(), Refused> {
        let network = match client.to_canonical() {
            IpAddr::V4(_) => &self.v4,
            IpAddr::V6(_) => &self.v6,
        };
        network
            .admit(&subnet_identity(client), now)
            .map_err(Refused::Capped)
    }
}

pub struct Gates {
    pub login: StartGate,
    pub link: StartGate,
    pub login_status: Clients,
    pub link_status: Clients,
    pub link_claim: Clients,
}

impl Gates {
    pub fn new(cfg: &SealedCfg) -> Self {
        Self {
            login: StartGate::new("/auth/login", cfg, FREE_LOGINS_PER_CLIENT),
            link: StartGate::new("/auth/link/create", cfg, FREE_LINKS_PER_CLIENT),
            login_status: Clients::new(LOGIN_POLLS_PER_CLIENT, AUTH_WINDOW),
            link_status: Clients::new(LINK_POLLS_PER_CLIENT, AUTH_WINDOW),
            link_claim: Clients::new(LINK_CLAIMS_PER_CLIENT, AUTH_WINDOW),
        }
    }
}
