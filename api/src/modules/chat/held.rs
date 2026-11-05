use std::time::{Duration, Instant};

pub const FRESH_SPREAD: f64 = 0.2;

#[derive(Clone, Debug)]
pub struct Held<T> {
    pub value: T,
    pub fresh_until: Instant,
}

impl<T> Held<T> {
    pub fn new(value: T, fresh_for: Duration) -> Self {
        Self {
            value,
            fresh_until: Instant::now()
                + fresh_for.mul_f64(1.0 - FRESH_SPREAD * rand::random::<f64>()),
        }
    }

    pub fn fresh(&self) -> bool {
        Instant::now() < self.fresh_until
    }

    pub fn map<U>(self, change: impl FnOnce(T) -> U) -> Held<U> {
        Held {
            value: change(self.value),
            fresh_until: self.fresh_until,
        }
    }
}
