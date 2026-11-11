use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use futures::future::BoxFuture;

use super::{SpendError, SpendStore};

#[derive(Default)]
pub(crate) struct MemorySpend {
    counters: Mutex<BTreeMap<String, u64>>,
    down: AtomicBool,
    reads: AtomicUsize,
}

impl MemorySpend {
    pub fn counters(&self) -> BTreeMap<String, u64> {
        self.counters.lock().unwrap().clone()
    }

    pub fn set(&self, key: &str, micro_usd: u64) {
        self.counters
            .lock()
            .unwrap()
            .insert(key.to_owned(), micro_usd);
    }

    pub fn go_down(&self) {
        self.down.store(true, Ordering::SeqCst);
    }

    pub fn come_back(&self) {
        self.down.store(false, Ordering::SeqCst);
    }

    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }

    fn reachable(&self) -> Result<(), SpendError> {
        match self.down.load(Ordering::SeqCst) {
            true => Err(SpendError::Unreachable("down".to_owned())),
            false => Ok(()),
        }
    }
}

impl SpendStore for MemorySpend {
    fn spent(&self, key: String) -> BoxFuture<'_, Result<u64, SpendError>> {
        Box::pin(async move {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.reachable()?;
            Ok(self
                .counters
                .lock()
                .unwrap()
                .get(&key)
                .copied()
                .unwrap_or(0))
        })
    }

    fn add(&self, key: String, micro_usd: u64) -> BoxFuture<'_, Result<u64, SpendError>> {
        Box::pin(async move {
            self.reachable()?;
            let mut counters = self.counters.lock().unwrap();
            let total = counters.entry(key).or_insert(0);
            *total += micro_usd;
            Ok(*total)
        })
    }
}
