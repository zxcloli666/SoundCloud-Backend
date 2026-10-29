use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use mini_moka::sync::Cache;

const WINDOW: Duration = Duration::from_secs(3600);
const BUNDLES_PER_WINDOW: u32 = 20;
const EVENTS_PER_WINDOW: u32 = 120;

pub struct Limits {
    bundles: Cache<String, Arc<AtomicU32>>,
    events: Cache<String, Arc<AtomicU32>>,
}

impl Limits {
    pub fn new() -> Self {
        let cache = || {
            Cache::builder()
                .time_to_live(WINDOW)
                .max_capacity(200_000)
                .build()
        };
        Self {
            bundles: cache(),
            events: cache(),
        }
    }

    pub fn bundle(&self, ip: IpAddr) -> bool {
        take(&self.bundles, ip, 1, BUNDLES_PER_WINDOW)
    }

    pub fn events(&self, ip: IpAddr, count: u32) -> bool {
        take(&self.events, ip, count, EVENTS_PER_WINDOW)
    }
}

fn take(cache: &Cache<String, Arc<AtomicU32>>, ip: IpAddr, count: u32, max: u32) -> bool {
    let key = client_key(ip);
    let counter = match cache.get(&key) {
        Some(counter) => counter,
        None => {
            let counter = Arc::new(AtomicU32::new(0));
            cache.insert(key, counter.clone());
            counter
        }
    };
    let before = counter.fetch_add(count, Ordering::Relaxed);
    before.saturating_add(count) <= max
}

fn client_key(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => format!("{:x?}", &v6.octets()[..8]),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_twenty_first_bundle_in_an_hour_is_refused() {
        let limits = Limits::new();
        let ip: IpAddr = "203.0.113.7".parse().expect("ip");

        for _ in 0..20 {
            assert!(limits.bundle(ip));
        }
        assert!(!limits.bundle(ip));
        assert!(limits.bundle("203.0.113.8".parse().expect("ip")));
    }

    #[test]
    fn events_count_one_by_one() {
        let limits = Limits::new();
        let ip: IpAddr = "203.0.113.7".parse().expect("ip");

        assert!(limits.events(ip, 100));
        assert!(limits.events(ip, 20));
        assert!(!limits.events(ip, 1));
    }
}
