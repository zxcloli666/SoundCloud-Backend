use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash, RandomState};
use std::sync::{Mutex, MutexGuard, PoisonError};

pub const SHARDS: usize = 64;

pub struct Shards<K, V> {
    maps: Vec<Mutex<HashMap<K, V>>>,
    hasher: RandomState,
}

impl<K: Hash + Eq, V> Shards<K, V> {
    pub fn new() -> Self {
        Self {
            maps: (0..SHARDS).map(|_| Mutex::new(HashMap::new())).collect(),
            hasher: RandomState::new(),
        }
    }

    pub fn lock<Q>(&self, key: &Q) -> MutexGuard<'_, HashMap<K, V>>
    where
        K: Borrow<Q>,
        Q: Hash + ?Sized,
    {
        let index = (self.hasher.hash_one(key) as usize) % SHARDS;
        self.lock_at(index)
    }

    pub fn lock_at(&self, index: usize) -> MutexGuard<'_, HashMap<K, V>> {
        self.maps[index % SHARDS]
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl<K: Hash + Eq, V> Default for Shards<K, V> {
    fn default() -> Self {
        Self::new()
    }
}
