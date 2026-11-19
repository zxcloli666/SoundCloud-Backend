use std::time::Instant;

use axum::http::StatusCode;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use uuid::Uuid;

use crate::common::session::session_hash;
use crate::common::windows::{AUTH_WINDOW, Clients};
use crate::error::{AppError, AppResult};

const STARTS: u32 = 6;
const CALLBACKS: u32 = 12;
const POLLS: u32 = 120;
const CHANGES: u32 = 20;
const READS: u32 = 60;
const FALLBACK_RETRY_SECONDS: i64 = 60;

#[derive(Clone, Copy)]
pub(super) enum Action {
    Start,
    Callback,
    Poll,
    Change,
    Read,
}

pub(super) struct Limits {
    starts: Clients,
    callbacks: Clients,
    polls: Clients,
    changes: Clients,
    reads: Clients,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            starts: Clients::new(STARTS, AUTH_WINDOW),
            callbacks: Clients::new(CALLBACKS, AUTH_WINDOW),
            polls: Clients::new(POLLS, AUTH_WINDOW),
            changes: Clients::new(CHANGES, AUTH_WINDOW),
            reads: Clients::new(READS, AUTH_WINDOW),
        }
    }
}

impl Limits {
    pub(super) fn admit(&self, action: Action, session_id: Uuid) -> AppResult<()> {
        let clients = match action {
            Action::Start => &self.starts,
            Action::Callback => &self.callbacks,
            Action::Poll => &self.polls,
            Action::Change => &self.changes,
            Action::Read => &self.reads,
        };
        let key = URL_SAFE_NO_PAD.encode(session_hash(session_id));
        clients.admit(&key, Instant::now()).map_err(|retry_ms| {
            let seconds = i64::try_from(retry_ms.div_ceil(1000)).unwrap_or(FALLBACK_RETRY_SECONDS);
            AppError::coded(
                StatusCode::TOO_MANY_REQUESTS,
                "mobile_rate_limited",
                "Too many mobile profile requests, retry shortly",
            )
            .with_retry_after(seconds)
        })
    }
}
