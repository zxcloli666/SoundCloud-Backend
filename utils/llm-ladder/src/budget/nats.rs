use std::hash::{BuildHasher, DefaultHasher, Hash, Hasher, RandomState};
use std::time::Duration;

use async_nats::jetstream::Context;
use async_nats::jetstream::kv::{self, CreateErrorKind, Operation, UpdateErrorKind};
use async_nats::jetstream::stream::StorageType;
use bytes::Bytes;
use futures::future::BoxFuture;
use tokio::sync::{Mutex, OnceCell};

use super::{SpendError, SpendStore};

const COUNTER_AGE: Duration = Duration::from_secs(40 * 24 * 3600);
const BUCKET_BYTES: i64 = 256 * 1024 * 1024;
const MAX_COUNTER_MESSAGE: i32 = 256;
const WRITE_TRIES: u32 = 24;
const WRITE_LANES: usize = 16;
const FIRST_BACKOFF_MICROS: u64 = 2_000;
const LONGEST_BACKOFF_MICROS: u64 = 200_000;

pub(crate) struct NatsSpend {
    jetstream: Context,
    bucket: String,
    store: OnceCell<kv::Store>,
    lanes: [Mutex<()>; WRITE_LANES],
}

impl NatsSpend {
    pub fn new(jetstream: Context, bucket: &str) -> Self {
        Self {
            jetstream,
            bucket: bucket.to_owned(),
            store: OnceCell::new(),
            lanes: std::array::from_fn(|_| Mutex::new(())),
        }
    }

    async fn store(&self) -> Result<&kv::Store, SpendError> {
        self.store.get_or_try_init(|| self.open()).await
    }

    async fn open(&self) -> Result<kv::Store, SpendError> {
        if let Ok(store) = self.jetstream.get_key_value(&self.bucket).await {
            return Ok(store);
        }
        let created = self
            .jetstream
            .create_key_value(kv::Config {
                bucket: self.bucket.clone(),
                history: 1,
                max_age: COUNTER_AGE,
                max_bytes: BUCKET_BYTES,
                max_value_size: MAX_COUNTER_MESSAGE,
                storage: StorageType::File,
                ..Default::default()
            })
            .await;
        match created {
            Ok(store) => Ok(store),
            Err(error) => self
                .jetstream
                .get_key_value(&self.bucket)
                .await
                .map_err(|_| unreachable(error)),
        }
    }

    async fn write(&self, key: &str, micro_usd: u64) -> Result<u64, SpendError> {
        let store = self.store().await?;
        let _own_writes_in_turn = self.lane(key).lock().await;
        for attempt in 0..WRITE_TRIES {
            if let Some(total) = add_once(store, key, micro_usd).await? {
                return Ok(total);
            }
            back_off(attempt).await;
        }
        Err(SpendError::Contended)
    }

    fn lane(&self, key: &str) -> &Mutex<()> {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        &self.lanes[hasher.finish() as usize % WRITE_LANES]
    }
}

impl SpendStore for NatsSpend {
    fn spent(&self, key: String) -> BoxFuture<'_, Result<u64, SpendError>> {
        Box::pin(async move {
            let stored = self.store().await?.get(key).await.map_err(unreachable)?;
            stored.as_deref().map_or(Ok(0), decode)
        })
    }

    fn add(&self, key: String, micro_usd: u64) -> BoxFuture<'_, Result<u64, SpendError>> {
        Box::pin(async move { self.write(&key, micro_usd).await })
    }
}

async fn add_once(store: &kv::Store, key: &str, micro_usd: u64) -> Result<Option<u64>, SpendError> {
    let current = store.entry(key).await.map_err(unreachable)?;
    let Some(entry) = current.filter(|entry| entry.operation == Operation::Put) else {
        return match store.create(key, encode(micro_usd)).await {
            Ok(_) => Ok(Some(micro_usd)),
            Err(error) if error.kind() == CreateErrorKind::AlreadyExists => Ok(None),
            Err(error) => Err(unreachable(error)),
        };
    };
    let total = decode(&entry.value)?.saturating_add(micro_usd);
    match store.update(key, encode(total), entry.revision).await {
        Ok(_) => Ok(Some(total)),
        Err(error) if error.kind() == UpdateErrorKind::WrongLastRevision => Ok(None),
        Err(error) => Err(unreachable(error)),
    }
}

async fn back_off(attempt: u32) {
    let ceiling = (FIRST_BACKOFF_MICROS << attempt.min(8)).min(LONGEST_BACKOFF_MICROS);
    let wait = RandomState::new().hash_one(attempt) % ceiling;
    tokio::time::sleep(Duration::from_micros(wait)).await;
}

fn encode(micro_usd: u64) -> Bytes {
    Bytes::from(micro_usd.to_string())
}

fn decode(stored: &[u8]) -> Result<u64, SpendError> {
    std::str::from_utf8(stored)
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .ok_or(SpendError::Corrupt)
}

fn unreachable(error: impl std::error::Error) -> SpendError {
    SpendError::Unreachable(error.to_string())
}

#[cfg(test)]
#[path = "nats_tests.rs"]
mod tests;
