use std::collections::HashSet;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use super::{PublicToken, PublicTokenId};

#[derive(Default)]
pub struct TokenSnapshot {
    state: RwLock<SnapshotState>,
}

#[derive(Default)]
struct SnapshotState {
    tokens: Vec<PublicToken>,
    rejected: HashSet<PublicTokenId>,
    loaded_at: Option<Instant>,
    reload_after: Option<Instant>,
    reload_failures: u32,
}

impl TokenSnapshot {
    pub fn tokens_fresh_after(&self, cutoff: DateTime<Utc>) -> Vec<PublicToken> {
        let state = self.state.read().unwrap_or_else(|error| error.into_inner());
        state
            .tokens
            .iter()
            .filter(|token| token.expires_at > cutoff)
            .cloned()
            .collect()
    }

    pub fn reload_due(&self, max_age: Duration) -> bool {
        let state = self.state.read().unwrap_or_else(|error| error.into_inner());
        let fresh = state
            .loaded_at
            .is_some_and(|loaded_at| loaded_at.elapsed() <= max_age);
        !fresh
            && state
                .reload_after
                .is_none_or(|reload_after| Instant::now() >= reload_after)
    }

    pub fn replace(&self, tokens: Vec<PublicToken>) {
        self.replace_at(tokens, Instant::now());
    }

    pub fn reject(&self, rejected: PublicTokenId) {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let superseded = state.tokens.iter().any(|token| {
            token.oauth_app_id == rejected.oauth_app_id && token.generation != rejected.generation
        });
        if !superseded {
            state.rejected.insert(rejected);
        }
        state.tokens.retain(|token| token.id() != rejected);
    }

    pub fn rejected(&self) -> Vec<PublicTokenId> {
        let state = self.state.read().unwrap_or_else(|error| error.into_inner());
        state.rejected.iter().copied().collect()
    }

    pub fn resolve_rejection(&self, rejected: PublicTokenId) {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        state.rejected.remove(&rejected);
    }

    pub fn record_reload_failure(&self, minimum: Duration, maximum: Duration) {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        state.reload_failures = state.reload_failures.saturating_add(1);
        let shift = state.reload_failures.saturating_sub(1).min(31);
        let multiplier = 1_u32 << shift;
        let delay = minimum.saturating_mul(multiplier).min(maximum);
        state.reload_after = Instant::now().checked_add(delay);
    }

    fn replace_at(&self, mut tokens: Vec<PublicToken>, loaded_at: Instant) {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        state.rejected.retain(|rejected| {
            !tokens.iter().any(|token| {
                token.oauth_app_id == rejected.oauth_app_id
                    && token.generation != rejected.generation
            })
        });
        tokens.retain(|token| !state.rejected.contains(&token.id()));
        state.tokens = tokens;
        state.loaded_at = Some(loaded_at);
        state.reload_after = None;
        state.reload_failures = 0;
    }

    #[cfg(test)]
    pub fn replace_stale(&self, tokens: Vec<PublicToken>, age: Duration) {
        self.replace_at(tokens, Instant::now() - age);
    }
}
