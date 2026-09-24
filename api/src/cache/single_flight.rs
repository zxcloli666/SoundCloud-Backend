use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as SyncMutex, Weak};

use tokio::sync::Mutex;

pub struct SingleFlight {
    gate: Mutex<()>,
}

impl SingleFlight {
    pub fn new() -> Self {
        Self {
            gate: Mutex::new(()),
        }
    }

    pub async fn get_or_load<T, E, Read, ReadFuture, Load, LoadFuture>(
        &self,
        read: Read,
        load: Load,
    ) -> Result<T, E>
    where
        Read: Fn() -> ReadFuture,
        ReadFuture: Future<Output = Option<T>>,
        Load: FnOnce() -> LoadFuture,
        LoadFuture: Future<Output = Result<T, E>>,
    {
        if let Some(value) = read().await {
            return Ok(value);
        }

        let _guard = self.gate.lock().await;
        if let Some(value) = read().await {
            return Ok(value);
        }

        load().await
    }
}

impl Default for SingleFlight {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Coalesce<T> {
    gate: Mutex<()>,
    completed: SyncMutex<Option<(u64, T)>>,
    completions: AtomicU64,
}

impl<T: Clone> Coalesce<T> {
    pub fn new() -> Self {
        Self {
            gate: Mutex::new(()),
            completed: SyncMutex::new(None),
            completions: AtomicU64::new(0),
        }
    }

    pub async fn run<E, Load, LoadFuture>(&self, load: Load) -> Result<T, E>
    where
        Load: FnOnce() -> LoadFuture,
        LoadFuture: Future<Output = Result<T, E>>,
    {
        let arrived = self.completions.load(Ordering::Acquire);
        let _guard = self.gate.lock().await;
        if let Some(value) = self.completed_after(arrived) {
            return Ok(value);
        }
        let value = load().await?;
        self.publish(value.clone());
        Ok(value)
    }

    fn completed_after(&self, arrived: u64) -> Option<T> {
        let completed = self
            .completed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        completed
            .as_ref()
            .filter(|(generation, _)| *generation > arrived)
            .map(|(_, value)| value.clone())
    }

    fn publish(&self, value: T) {
        let mut completed = self
            .completed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = self.completions.fetch_add(1, Ordering::AcqRel) + 1;
        *completed = Some((generation, value));
    }
}

impl<T: Clone> Default for Coalesce<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct KeyedCoalesce<T> {
    flights: SyncMutex<HashMap<String, Weak<Coalesce<T>>>>,
}

impl<T: Clone> KeyedCoalesce<T> {
    pub fn new() -> Self {
        Self {
            flights: SyncMutex::new(HashMap::new()),
        }
    }

    pub async fn run<E, Load, LoadFuture>(&self, key: &str, load: Load) -> Result<T, E>
    where
        Load: FnOnce() -> LoadFuture,
        LoadFuture: Future<Output = Result<T, E>>,
    {
        let flight = self.flight(key);
        flight.flight.run(load).await
    }

    fn flight<'a>(&'a self, key: &'a str) -> CoalesceFlight<'a, T> {
        let mut flights = self
            .flights
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let flight = flights.get(key).and_then(Weak::upgrade).unwrap_or_else(|| {
            let flight = Arc::new(Coalesce::new());
            flights.insert(key.to_owned(), Arc::downgrade(&flight));
            flight
        });
        CoalesceFlight {
            owner: self,
            key,
            flight,
        }
    }
}

impl<T: Clone> Default for KeyedCoalesce<T> {
    fn default() -> Self {
        Self::new()
    }
}

struct CoalesceFlight<'a, T> {
    owner: &'a KeyedCoalesce<T>,
    key: &'a str,
    flight: Arc<Coalesce<T>>,
}

impl<T> Drop for CoalesceFlight<'_, T> {
    fn drop(&mut self) {
        let mut flights = self
            .owner
            .flights
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let is_last = flights.get(self.key).is_some_and(|current| {
            current.as_ptr() == Arc::as_ptr(&self.flight) && Arc::strong_count(&self.flight) == 1
        });
        if is_last {
            flights.remove(self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use tokio::sync::{Barrier, RwLock};
    use tokio::task::JoinSet;

    use super::*;

    #[tokio::test]
    async fn concurrent_misses_load_once() {
        let flight = Arc::new(SingleFlight::new());
        let cached = Arc::new(RwLock::new(None));
        let loads = Arc::new(AtomicUsize::new(0));
        let mut tasks = JoinSet::new();

        for _ in 0..32 {
            let flight = flight.clone();
            let cached = cached.clone();
            let loads = loads.clone();
            tasks.spawn(async move {
                flight
                    .get_or_load(
                        || {
                            let cached = cached.clone();
                            async move { *cached.read().await }
                        },
                        || {
                            let cached = cached.clone();
                            async move {
                                loads.fetch_add(1, Ordering::Relaxed);
                                tokio::time::sleep(Duration::from_millis(10)).await;
                                *cached.write().await = Some(42);
                                Ok::<_, ()>(42)
                            }
                        },
                    )
                    .await
            });
        }

        while let Some(result) = tasks.join_next().await {
            assert!(matches!(result, Ok(Ok(42))), "{result:?}");
        }

        assert_eq!(loads.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn keyed_coalesce_runs_one_load_for_everyone_waiting_on_the_same_key() {
        let flight = Arc::new(KeyedCoalesce::<u32>::new());
        let loads = Arc::new(AtomicUsize::new(0));
        let mut tasks = JoinSet::new();

        for _ in 0..32 {
            let flight = flight.clone();
            let loads = loads.clone();
            tasks.spawn(async move {
                flight
                    .run("rec:user:1", || {
                        let loads = loads.clone();
                        async move {
                            loads.fetch_add(1, Ordering::Relaxed);
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            Ok::<_, ()>(42)
                        }
                    })
                    .await
            });
        }

        while let Some(result) = tasks.join_next().await {
            assert!(matches!(result, Ok(Ok(42))), "{result:?}");
        }

        assert_eq!(
            loads.load(Ordering::Relaxed),
            1,
            "waiters on one key must reuse the leader's result instead of recomputing it"
        );
    }

    #[tokio::test]
    async fn a_caller_arriving_after_a_flight_finished_loads_again() {
        let flight = KeyedCoalesce::<u32>::new();
        let loads = AtomicUsize::new(0);
        let load = || async {
            loads.fetch_add(1, Ordering::Relaxed);
            Ok::<_, ()>(7)
        };

        assert_eq!(flight.run("k", load).await, Ok(7));
        assert_eq!(flight.run("k", load).await, Ok(7));

        assert_eq!(
            loads.load(Ordering::Relaxed),
            2,
            "a finished flight must not be reused as a cache by later callers"
        );
    }

    #[tokio::test]
    async fn a_finished_flight_keeps_nothing_alive() {
        let flight = KeyedCoalesce::<String>::new();
        let heavy = || async { Ok::<_, ()>("x".repeat(4096)) };

        assert!(flight.run("k", heavy).await.is_ok());

        let flights = flight
            .flights
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            flights.is_empty(),
            "a flight with no callers left must not keep its result or its map entry alive"
        );
    }

    #[tokio::test]
    async fn a_failed_flight_is_not_published_to_the_next_caller() {
        let flight = KeyedCoalesce::<u32>::new();

        assert_eq!(
            flight.run("k", || async { Err::<u32, _>(()) }).await,
            Err(())
        );
        assert_eq!(flight.run("k", || async { Ok::<_, ()>(5) }).await, Ok(5));
    }

    #[tokio::test]
    async fn keyed_flight_coalesces_concurrent_misses_for_same_key() {
        let flight = Arc::new(SingleFlight::new());
        let cached = Arc::new(RwLock::new(None));
        let loads = Arc::new(AtomicUsize::new(0));
        let mut tasks = JoinSet::new();

        for _ in 0..32 {
            let flight = flight.clone();
            let cached = cached.clone();
            let loads = loads.clone();
            tasks.spawn(async move {
                flight
                    .get_or_load(
                        || {
                            let cached = cached.clone();
                            async move { *cached.read().await }
                        },
                        || {
                            let cached = cached.clone();
                            async move {
                                loads.fetch_add(1, Ordering::Relaxed);
                                tokio::time::sleep(Duration::from_millis(10)).await;
                                *cached.write().await = Some(42);
                                Ok::<_, ()>(42)
                            }
                        },
                    )
                    .await
            });
        }

        while let Some(result) = tasks.join_next().await {
            assert!(matches!(result, Ok(Ok(42))), "{result:?}");
        }

        assert_eq!(loads.load(Ordering::Relaxed), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn keyed_flight_runs_different_keys_concurrently() {
        let flight = Arc::new(KeyedCoalesce::<u32>::new());
        let loaders = Arc::new(Barrier::new(2));

        let first = {
            let flight = flight.clone();
            let loaders = loaders.clone();
            tokio::spawn(async move {
                flight
                    .run("rec:user:1", || async move {
                        loaders.wait().await;
                        Ok::<_, ()>(1)
                    })
                    .await
            })
        };
        let second = tokio::spawn(async move {
            flight
                .run("rec:user:2", || async move {
                    loaders.wait().await;
                    Ok::<_, ()>(2)
                })
                .await
        });

        let result = tokio::time::timeout(Duration::from_secs(1), async {
            (first.await, second.await)
        })
        .await;

        assert!(matches!(result, Ok((Ok(Ok(1)), Ok(Ok(2))))));
    }
}
