use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use mini_moka::sync::Cache;
use uuid::Uuid;

use crate::modules::chat::held::Held;
use crate::modules::chat::model::DeviceRow;

const KEPT_FOR: Duration = Duration::from_secs(600);
const ROWS_BYTES: u64 = 16 * 1024 * 1024;
const USERS_BYTES: u64 = 16 * 1024 * 1024;
const ROW_OVERHEAD: usize = 256;

pub struct DeviceCache {
    rows: Cache<Uuid, Held<DeviceRow>>,
    users: Cache<String, Held<Arc<Vec<DeviceRow>>>>,
    writes: AtomicU64,
    fresh_for: Duration,
}

impl DeviceCache {
    pub fn new() -> Self {
        Self::with_freshness(KEPT_FOR)
    }

    pub fn with_freshness(fresh_for: Duration) -> Self {
        Self {
            rows: Cache::builder()
                .max_capacity(ROWS_BYTES)
                .weigher(|_, held: &Held<DeviceRow>| row_weight(&held.value))
                .time_to_live(KEPT_FOR)
                .build(),
            users: Cache::builder()
                .max_capacity(USERS_BYTES)
                .weigher(|user: &String, held: &Held<Arc<Vec<DeviceRow>>>| {
                    let bytes = user.len()
                        + held
                            .value
                            .iter()
                            .map(|row| row_weight(row) as usize)
                            .sum::<usize>();
                    u32::try_from(bytes).unwrap_or(u32::MAX)
                })
                .time_to_live(KEPT_FOR)
                .build(),
            writes: AtomicU64::new(0),
            fresh_for,
        }
    }

    pub fn bytes(&self) -> u64 {
        self.rows.weighted_size() + self.users.weighted_size()
    }

    pub fn row(&self, id: Uuid) -> Option<DeviceRow> {
        self.held_row(id).filter(Held::fresh).map(|held| held.value)
    }

    pub fn held_row(&self, id: Uuid) -> Option<Held<DeviceRow>> {
        self.rows.get(&id)
    }

    pub fn user(&self, user_id: &str) -> Option<Arc<Vec<DeviceRow>>> {
        self.held_user(user_id)
            .filter(Held::fresh)
            .map(|held| held.value)
    }

    pub fn held_user(&self, user_id: &str) -> Option<Held<Arc<Vec<DeviceRow>>>> {
        self.users.get(&user_id.to_owned())
    }

    pub fn stamp(&self) -> u64 {
        self.writes.load(Ordering::SeqCst)
    }

    pub fn keep_row(&self, stamp: u64, row: DeviceRow) {
        let id = row.id;
        self.rows.insert(id, Held::new(row, self.fresh_for));
        if self.stamp() != stamp {
            self.rows.invalidate(&id);
        }
    }

    pub fn keep_user(&self, stamp: u64, user_id: &str, rows: Arc<Vec<DeviceRow>>) {
        self.users
            .insert(user_id.to_owned(), Held::new(rows, self.fresh_for));
        if self.stamp() != stamp {
            self.users.invalidate(&user_id.to_owned());
        }
    }

    pub fn seen(&self, stamp: u64, row: DeviceRow) {
        self.users.invalidate(&row.user_id);
        self.keep_row(stamp, row);
    }

    pub fn forget(&self, user_id: &str, ids: &[Uuid]) {
        self.writes.fetch_add(1, Ordering::SeqCst);
        for id in ids {
            self.rows.invalidate(id);
        }
        self.users.invalidate(&user_id.to_owned());
    }
}

impl Default for DeviceCache {
    fn default() -> Self {
        Self::new()
    }
}

fn row_weight(row: &DeviceRow) -> u32 {
    let text = row.user_id.len()
        + row.status.len()
        + row.platform.len()
        + row.name.len()
        + row.app_version.len()
        + row.tier.len()
        + row.caps.iter().map(String::len).sum::<usize>();
    let bytes = ROW_OVERHEAD + text + row.auth_pub.len() + row.box_pub.len();
    u32::try_from(bytes).unwrap_or(u32::MAX)
}

#[cfg(test)]
#[path = "../tests/device_cache_tests.rs"]
mod tests;
