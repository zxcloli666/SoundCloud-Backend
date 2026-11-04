use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use backend_contracts::remote::MAX_DEVICES_PER_USER;
use tokio::sync::Notify;

#[derive(Default)]
pub struct Waits {
    held: Mutex<HashMap<String, HashMap<String, Arc<Notify>>>>,
}

pub struct Slot {
    pub replaced: Arc<Notify>,
    waits: Arc<Waits>,
    user: String,
    device: String,
}

impl Waits {
    pub fn enter(self: &Arc<Self>, user: &str, device: &str) -> Option<Slot> {
        let replaced = Arc::new(Notify::new());
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        let devices = held.entry(user.to_owned()).or_default();
        if !devices.contains_key(device) && devices.len() >= MAX_DEVICES_PER_USER {
            return None;
        }
        if let Some(previous) = devices.insert(device.to_owned(), replaced.clone()) {
            previous.notify_one();
        }
        Some(Slot {
            replaced,
            waits: self.clone(),
            user: user.to_owned(),
            device: device.to_owned(),
        })
    }

    #[cfg(test)]
    pub fn count(&self, user: &str) -> usize {
        let held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        held.get(user).map_or(0, HashMap::len)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut held = self.waits.held.lock().unwrap_or_else(|e| e.into_inner());
        let Some(devices) = held.get_mut(&self.user) else {
            return;
        };
        if devices
            .get(&self.device)
            .is_some_and(|current| Arc::ptr_eq(current, &self.replaced))
        {
            devices.remove(&self.device);
        }
        if devices.is_empty() {
            held.remove(&self.user);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_holds_at_most_three_devices() {
        let waits = Arc::new(Waits::default());
        let first = waits.enter("1", "a").unwrap();
        let _second = waits.enter("1", "b").unwrap();
        let _third = waits.enter("1", "c").unwrap();
        assert!(waits.enter("1", "d").is_none());
        assert!(waits.enter("2", "d").is_some());
        drop(first);
        assert!(waits.enter("1", "d").is_some());
    }

    #[tokio::test]
    async fn a_second_wait_from_the_same_device_replaces_the_first() {
        let waits = Arc::new(Waits::default());
        let first = waits.enter("1", "a").unwrap();
        let second = waits.enter("1", "a").unwrap();
        assert_eq!(waits.count("1"), 1);
        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            first.replaced.notified(),
        )
        .await
        .expect("the first wait is told to stop");
        drop(first);
        assert_eq!(waits.count("1"), 1, "the replaced wait leaves the newer one in place");
        drop(second);
        assert_eq!(waits.count("1"), 0);
    }
}
