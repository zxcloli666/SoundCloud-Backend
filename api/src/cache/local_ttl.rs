use std::sync::RwLock;
use std::time::{Duration, Instant};

struct Entry<T> {
    value: T,
    expires_at: Instant,
}

pub struct LocalTtlCache<T> {
    ttl: Duration,
    entry: RwLock<Option<Entry<T>>>,
}

impl<T: Clone> LocalTtlCache<T> {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entry: RwLock::new(None),
        }
    }

    pub fn get(&self) -> Option<T> {
        self.get_at(Instant::now())
    }

    pub fn set(&self, value: T) {
        self.set_at(value, Instant::now());
    }

    fn get_at(&self, now: Instant) -> Option<T> {
        let entry = self
            .entry
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = entry.as_ref()?;
        (now < entry.expires_at).then(|| entry.value.clone())
    }

    fn set_at(&self, value: T, now: Instant) {
        let mut entry = self
            .entry
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *entry = Some(Entry {
            value,
            expires_at: now + self.ttl,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_returns_value_before_expiration() {
        let cache = LocalTtlCache::new(Duration::from_secs(30));
        let now = Instant::now();
        cache.set_at(42, now);

        assert_eq!(cache.get_at(now + Duration::from_secs(29)), Some(42));
    }

    #[test]
    fn get_returns_none_at_expiration() {
        let cache = LocalTtlCache::new(Duration::from_secs(30));
        let now = Instant::now();
        cache.set_at(42, now);

        assert_eq!(cache.get_at(now + Duration::from_secs(30)), None);
    }

    #[test]
    fn poisoned_lock_recovers_on_next_write() {
        let cache = LocalTtlCache::new(Duration::from_secs(30));
        let poisoned = std::panic::catch_unwind(|| {
            let _guard = cache.entry.write().unwrap();
            panic!("poison local cache");
        });

        assert!(poisoned.is_err());
        cache.set(42);
        assert_eq!(cache.get(), Some(42));
    }
}
