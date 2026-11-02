use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

pub const ESCALATION_USERS: usize = 3;
const ESCALATION_WINDOW: Duration = Duration::from_secs(10 * 60);

const LEGACY_GROUPS: [(&str, &[&str]); 4] = [
    (
        "likes",
        &[
            "like_track",
            "unlike_track",
            "like_playlist",
            "unlike_playlist",
        ],
    ),
    ("follows", &["follow_user", "unfollow_user"]),
    (
        "playlists",
        &[
            "playlist_create",
            "playlist_delete",
            "playlist_membership",
            "playlist_update",
        ],
    ),
    ("comments", &["comment"]),
];

pub fn actions_of(saved: &str) -> Vec<String> {
    match LEGACY_GROUPS.iter().find(|(group, _)| *group == saved) {
        Some((_, members)) => members.iter().map(|action| (*action).to_owned()).collect(),
        None => vec![saved.to_owned()],
    }
}

pub fn legacy_group(action_type: &str) -> Option<&'static str> {
    LEGACY_GROUPS
        .iter()
        .find(|(_, members)| members.contains(&action_type))
        .map(|(group, _)| *group)
}

pub struct AppReports<K> {
    reported: Mutex<HashMap<K, HashMap<String, Instant>>>,
}

impl<K> Default for AppReports<K> {
    fn default() -> Self {
        Self {
            reported: Mutex::new(HashMap::new()),
        }
    }
}

impl<K: Eq + Hash> AppReports<K> {
    pub fn record(&self, key: K, user_id: &str) -> usize {
        self.record_at(key, user_id, Instant::now())
    }

    pub fn succeeded(&self, key: &K) {
        self.reported
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(key);
    }

    fn record_at(&self, key: K, user_id: &str, now: Instant) -> usize {
        let mut reported = self.reported.lock().unwrap_or_else(PoisonError::into_inner);
        let users = reported.entry(key).or_default();
        users.retain(|_, seen| now.saturating_duration_since(*seen) < ESCALATION_WINDOW);
        users.insert(user_id.to_owned(), now);
        users.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_group_name_expands_to_its_actions_and_an_action_stays_alone() {
        assert_eq!(actions_of("likes").len(), 4);
        assert_eq!(actions_of("follows"), ["follow_user", "unfollow_user"]);
        assert_eq!(actions_of("like_track"), ["like_track"]);
        assert_eq!(actions_of("track_update"), ["track_update"]);
    }

    #[test]
    fn every_grouped_action_knows_its_legacy_group() {
        assert_eq!(legacy_group("unlike_playlist"), Some("likes"));
        assert_eq!(legacy_group("playlist_membership"), Some("playlists"));
        assert_eq!(legacy_group("track_update"), None);
    }

    #[test]
    fn a_success_clears_the_reports_of_its_key_only() {
        let reports = AppReports::default();
        reports.record("likes", "1");
        reports.record("likes", "2");
        reports.record("follows", "1");

        reports.succeeded(&"likes");

        assert_eq!(reports.record("likes", "3"), 1);
        assert_eq!(reports.record("follows", "2"), 2);
    }

    #[test]
    fn the_same_user_counts_once() {
        let reports = AppReports::default();

        assert_eq!(reports.record("likes", "1"), 1);
        assert_eq!(reports.record("likes", "1"), 1);
        assert_eq!(reports.record("likes", "2"), 2);
        assert_eq!(reports.record("follows", "3"), 1);
    }

    #[test]
    fn reports_older_than_ten_minutes_drop_out() {
        let reports = AppReports::default();
        let start = Instant::now();

        reports.record_at("likes", "1", start);
        reports.record_at("likes", "2", start + Duration::from_secs(120));

        assert_eq!(
            reports.record_at("likes", "3", start + Duration::from_secs(9 * 60)),
            3
        );
        assert_eq!(
            reports.record_at("likes", "4", start + Duration::from_secs(11 * 60)),
            3
        );
    }
}
