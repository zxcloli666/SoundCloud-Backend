use std::net::{IpAddr, SocketAddr};
use std::time::Instant;

use axum::extract::{ConnectInfo, Request};
use ipnet::IpNet;

use crate::common::admission::{client_address, client_identity};
use crate::common::windows::{Clients, Window};
use crate::modules::chat::config::StartLimits;
use crate::modules::chat::errors::BusySource;
use crate::modules::chat::{ChatError, ChatResult};

pub const LOGINS_PER_WINDOW: u32 = 3_000;
pub const LINKS_PER_WINDOW: u32 = 3_000;

#[cfg(test)]
pub use crate::common::windows::CLIENTS_TRACKED;

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
