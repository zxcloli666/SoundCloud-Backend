use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use uuid::Uuid;

#[derive(Default)]
pub struct WritePauses {
    until: Mutex<HashMap<Uuid, Instant>>,
}

impl WritePauses {
    pub fn pause(&self, oauth_app_id: Uuid, seconds: i64) {
        let seconds = u64::try_from(seconds).unwrap_or(0).max(1);
        let resume_at = Instant::now() + Duration::from_secs(seconds);
        let mut until = self.until.lock().unwrap_or_else(PoisonError::into_inner);
        let paused_until = until.entry(oauth_app_id).or_insert(resume_at);
        *paused_until = (*paused_until).max(resume_at);
    }

    pub fn remaining_seconds(&self, oauth_app_id: Uuid) -> Option<i64> {
        let mut until = self.until.lock().unwrap_or_else(PoisonError::into_inner);
        let remaining = until
            .get(&oauth_app_id)?
            .saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            until.remove(&oauth_app_id);
            return None;
        }
        Some(
            i64::try_from(remaining.as_secs())
                .unwrap_or(i64::MAX)
                .max(1),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paused_app_reports_the_wait_and_other_apps_stay_open() {
        let pauses = WritePauses::default();
        let paused = Uuid::now_v7();

        pauses.pause(paused, 300);

        assert!(
            pauses
                .remaining_seconds(paused)
                .is_some_and(|seconds| seconds > 290)
        );
        assert_eq!(pauses.remaining_seconds(Uuid::now_v7()), None);
    }

    #[test]
    fn a_shorter_pause_never_shortens_a_longer_one() {
        let pauses = WritePauses::default();
        let app = Uuid::now_v7();

        pauses.pause(app, 600);
        pauses.pause(app, 5);

        assert!(
            pauses
                .remaining_seconds(app)
                .is_some_and(|seconds| seconds > 590)
        );
    }
}
