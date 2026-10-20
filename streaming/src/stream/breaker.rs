use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

const FAILURES_TO_OPEN: u32 = 3;
const OPEN_FOR_MS: u64 = 60_000;

#[derive(Default)]
pub struct Breaker {
    failures: AtomicU32,
    open_until: AtomicU64,
}

impl Breaker {
    pub fn is_open(&self) -> bool {
        let until = self.open_until.load(Ordering::Relaxed);
        until > 0 && now_ms() < until
    }

    pub fn succeed(&self) {
        self.failures.store(0, Ordering::Relaxed);
        self.open_until.store(0, Ordering::Relaxed);
    }

    pub fn fail(&self) -> Option<u32> {
        let failures = self.failures.fetch_add(1, Ordering::Relaxed) + 1;
        if failures < FAILURES_TO_OPEN || self.is_open() {
            return None;
        }
        self.open_until
            .store(now_ms() + OPEN_FOR_MS, Ordering::Relaxed);
        Some(failures)
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::Breaker;

    #[test]
    fn three_failures_in_a_row_open_it_and_one_success_closes_it() {
        let breaker = Breaker::default();
        assert_eq!(breaker.fail(), None);
        assert_eq!(breaker.fail(), None);
        assert!(!breaker.is_open());

        assert_eq!(breaker.fail(), Some(3));
        assert!(breaker.is_open());
        assert_eq!(breaker.fail(), None, "an open breaker is not opened again");

        breaker.succeed();
        assert!(!breaker.is_open());
        assert_eq!(breaker.fail(), None, "a success starts the count over");
    }
}
