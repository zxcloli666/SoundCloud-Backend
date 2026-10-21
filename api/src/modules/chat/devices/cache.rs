use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use mini_moka::sync::Cache;
use uuid::Uuid;

use crate::modules::chat::model::DeviceRow;

const KEPT_FOR: Duration = Duration::from_secs(600);
const ROWS_BYTES: u64 = 16 * 1024 * 1024;
const USERS_BYTES: u64 = 16 * 1024 * 1024;
const ROW_OVERHEAD: usize = 256;

pub struct DeviceCache {
    rows: Cache<Uuid, DeviceRow>,
    users: Cache<String, Arc<Vec<DeviceRow>>>,
    writes: AtomicU64,
}

impl DeviceCache {
    pub fn new() -> Self {
        Self {
            rows: Cache::builder()
                .max_capacity(ROWS_BYTES)
                .weigher(|_, row: &DeviceRow| row_weight(row))
                .time_to_live(KEPT_FOR)
                .build(),
            users: Cache::builder()
                .max_capacity(USERS_BYTES)
                .weigher(|user: &String, rows: &Arc<Vec<DeviceRow>>| {
                    let bytes = user.len()
                        + rows
                            .iter()
                            .map(|row| row_weight(row) as usize)
                            .sum::<usize>();
                    u32::try_from(bytes).unwrap_or(u32::MAX)
                })
                .time_to_live(KEPT_FOR)
                .build(),
            writes: AtomicU64::new(0),
        }
    }

    pub fn row(&self, id: Uuid) -> Option<DeviceRow> {
        self.rows.get(&id)
    }

    pub fn user(&self, user_id: &str) -> Option<Arc<Vec<DeviceRow>>> {
        self.users.get(&user_id.to_owned())
    }

    pub fn stamp(&self) -> u64 {
        self.writes.load(Ordering::SeqCst)
    }

    pub fn keep_row(&self, stamp: u64, row: DeviceRow) {
        let id = row.id;
        self.rows.insert(id, row);
        if self.stamp() != stamp {
            self.rows.invalidate(&id);
        }
    }

    pub fn keep_user(&self, stamp: u64, user_id: &str, rows: Arc<Vec<DeviceRow>>) {
        self.users.insert(user_id.to_owned(), rows);
        if self.stamp() != stamp {
            self.users.invalidate(&user_id.to_owned());
        }
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
