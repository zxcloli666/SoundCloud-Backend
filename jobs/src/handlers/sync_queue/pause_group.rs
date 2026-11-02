use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

pub const ESCALATION_USERS: usize = 3;
const ESCALATION_WINDOW: Duration = Duration::from_secs(10 * 60);

const LIKES: [&str; 4] = [
    "like_track",
    "unlike_track",
    "like_playlist",
    "unlike_playlist",
];
const FOLLOWS: [&str; 2] = ["follow_user", "unfollow_user"];
const PLAYLISTS: [&str; 4] = [
    "playlist_create",
    "playlist_delete",
    "playlist_membership",
    "playlist_update",
];

pub fn group(action_type: &str) -> String {
    if LIKES.contains(&action_type) {
        "likes".to_owned()
    } else if FOLLOWS.contains(&action_type) {
        "follows".to_owned()
    } else if action_type.starts_with("playlist_") {
        "playlists".to_owned()
    } else if action_type == "comment" {
        "comments".to_owned()
    } else {
        action_type.to_owned()
    }
}

pub fn members(group: &str) -> Vec<String> {
    let names: &[&str] = match group {
        "likes" => &LIKES,
        "follows" => &FOLLOWS,
        "playlists" => &PLAYLISTS,
        "comments" => &["comment"],
        other => &[other],
    };
    names.iter().map(|name| (*name).to_owned()).collect()
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
    fn actions_fall_into_their_groups() {
        assert_eq!(group("like_track"), "likes");
        assert_eq!(group("unlike_playlist"), "likes");
        assert_eq!(group("unfollow_user"), "follows");
        assert_eq!(group("playlist_create"), "playlists");
        assert_eq!(group("playlist_membership"), "playlists");
        assert_eq!(group("playlist_update"), "playlists");
        assert_eq!(group("comment"), "comments");
        assert_eq!(group("track_update"), "track_update");
    }

    #[test]
    fn every_member_maps_back_to_its_group() {
        for name in ["likes", "follows", "playlists", "comments", "track_delete"] {
            for member in members(name) {
                assert_eq!(group(&member), name);
            }
        }
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
