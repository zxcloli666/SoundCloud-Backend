use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use super::DeviceCache;
use crate::modules::chat::model::DeviceRow;

fn row(user: &str) -> DeviceRow {
    let now = Utc::now();
    DeviceRow {
        id: Uuid::new_v4(),
        user_id: user.to_owned(),
        session_id: Uuid::new_v4(),
        status: "active".to_owned(),
        auth_pub: vec![1; 32],
        box_pub: vec![2; 32],
        platform: "macos".to_owned(),
        name: "MacBook".to_owned(),
        app_version: "8.7.0".to_owned(),
        caps: vec![],
        tier: "direct".to_owned(),
        created_at: now,
        history_unlocked_at: None,
        last_seen_at: now,
        revoked_at: None,
    }
}

#[test]
fn a_read_finished_after_a_forget_is_not_kept() {
    let cache = DeviceCache::new();
    let device = row("101");
    let stamp = cache.stamp();
    cache.forget("101", &[device.id]);
    cache.keep_row(stamp, device.clone());
    cache.keep_user(stamp, "101", Arc::new(vec![device.clone()]));
    assert!(cache.row(device.id).is_none());
    assert!(cache.user("101").is_none());
}

#[test]
fn a_read_with_no_write_in_between_is_kept_until_forgotten() {
    let cache = DeviceCache::new();
    let device = row("101");
    let other = row("202");
    let stamp = cache.stamp();
    cache.keep_row(stamp, device.clone());
    cache.keep_row(stamp, other.clone());
    cache.keep_user(stamp, "101", Arc::new(vec![device.clone()]));
    assert_eq!(cache.row(device.id).map(|kept| kept.id), Some(device.id));
    assert_eq!(cache.user("101").map(|kept| kept.len()), Some(1));

    cache.forget("101", &[device.id]);
    assert!(cache.row(device.id).is_none());
    assert!(cache.user("101").is_none());
    assert!(cache.row(other.id).is_some(), "other devices stay cached");
}
