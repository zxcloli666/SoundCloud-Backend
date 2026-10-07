use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::http::StatusCode;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::error::{AppError, AppResult};

const CONCURRENT_UPLOADS: usize = 4;
const BUSY_RETRY_AFTER_SEC: i64 = 30;

pub struct UploadGate {
    busy_users: Mutex<HashSet<String>>,
    slots: Arc<Semaphore>,
}

pub struct UploadTicket<'a> {
    gate: &'a UploadGate,
    user: String,
    _slot: OwnedSemaphorePermit,
}

impl UploadGate {
    pub fn new() -> Self {
        Self::with_slots(CONCURRENT_UPLOADS)
    }

    fn with_slots(slots: usize) -> Self {
        Self {
            busy_users: Mutex::new(HashSet::new()),
            slots: Arc::new(Semaphore::new(slots)),
        }
    }

    pub fn enter(&self, user: &str) -> AppResult<UploadTicket<'_>> {
        let mut busy = self
            .busy_users
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if busy.contains(user) {
            return Err(AppError::coded(
                StatusCode::CONFLICT,
                "upload_in_progress",
                "Another upload from this account is still running",
            ));
        }
        let slot = self.slots.clone().try_acquire_owned().map_err(|_| {
            AppError::coded(
                StatusCode::SERVICE_UNAVAILABLE,
                "uploads_busy",
                "Too many uploads right now, retry shortly",
            )
            .with_retry_after(BUSY_RETRY_AFTER_SEC)
        })?;
        busy.insert(user.to_owned());
        Ok(UploadTicket {
            gate: self,
            user: user.to_owned(),
            _slot: slot,
        })
    }
}

impl Default for UploadGate {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for UploadTicket<'_> {
    fn drop(&mut self) {
        self.gate
            .busy_users
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.user);
    }
}

#[cfg(test)]
mod tests {
    use super::UploadGate;

    #[test]
    fn one_account_uploads_one_file_at_a_time() {
        let gate = UploadGate::new();
        let first = gate.enter("1").expect("first upload");
        assert!(gate.enter("1").is_err());
        assert!(gate.enter("2").is_ok());
        drop(first);
        assert!(gate.enter("1").is_ok());
    }

    #[test]
    fn uploads_beyond_the_slots_are_turned_away() {
        let gate = UploadGate::with_slots(1);
        let held = gate.enter("1").expect("first upload");
        assert!(gate.enter("2").is_err());
        drop(held);
        assert!(gate.enter("2").is_ok());
    }
}
