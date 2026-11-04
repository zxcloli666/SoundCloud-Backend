use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use hdrhistogram::Histogram;
use serde::Serialize;
use tokio::time::Instant;

pub const LATENCY_MAX_MS: u64 = 3_600_000;

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct Tick {
    pub t_ms: u64,
    pub online: i64,
    pub parked: i64,
    pub ok: u64,
    pub errors: u64,
    pub sent: u64,
    pub received: u64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq)]
pub struct Spread {
    pub count: u64,
    pub p50_ms: u64,
    pub p90_ms: u64,
    pub p99_ms: u64,
    pub max_ms: u64,
}

pub struct RouteStats {
    pub name: String,
    pub requests: AtomicU64,
    pub ok: AtomicU64,
    pub errors: AtomicU64,
    pub sent: AtomicU64,
    pub received: AtomicU64,
    pub unreadable: AtomicU64,
    pub online: AtomicI64,
    pub parked: AtomicI64,
    kinds: Mutex<BTreeMap<String, u64>>,
    wake: Mutex<Histogram<u64>>,
    catchup: Mutex<Histogram<u64>>,
    calls: Mutex<Histogram<u64>>,
}

pub struct Stats {
    started: Instant,
    pub routes: Vec<RouteStats>,
    timeline: Mutex<Vec<Tick>>,
    restart_ms: AtomicU64,
    recovered: Mutex<Vec<u64>>,
}

pub fn histogram() -> Histogram<u64> {
    Histogram::new_with_bounds(1, LATENCY_MAX_MS, 3).expect("valid histogram bounds")
}

fn record(histogram: &Mutex<Histogram<u64>>, ms: u64) {
    if let Ok(mut histogram) = histogram.lock() {
        histogram.saturating_record(ms.clamp(1, LATENCY_MAX_MS));
    }
}

pub fn spread(histogram: &Histogram<u64>) -> Spread {
    if histogram.is_empty() {
        return Spread::default();
    }
    Spread {
        count: histogram.len(),
        p50_ms: histogram.value_at_quantile(0.50),
        p90_ms: histogram.value_at_quantile(0.90),
        p99_ms: histogram.value_at_quantile(0.99),
        max_ms: histogram.max(),
    }
}

impl RouteStats {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            requests: AtomicU64::new(0),
            ok: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            sent: AtomicU64::new(0),
            received: AtomicU64::new(0),
            unreadable: AtomicU64::new(0),
            online: AtomicI64::new(0),
            parked: AtomicI64::new(0),
            kinds: Mutex::new(BTreeMap::new()),
            wake: Mutex::new(histogram()),
            catchup: Mutex::new(histogram()),
            calls: Mutex::new(histogram()),
        }
    }

    pub fn kinds(&self) -> BTreeMap<String, u64> {
        self.kinds
            .lock()
            .map(|kinds| kinds.clone())
            .unwrap_or_default()
    }

    pub fn wake(&self) -> Histogram<u64> {
        self.wake
            .lock()
            .map(|h| h.clone())
            .unwrap_or_else(|_| histogram())
    }

    pub fn catchup(&self) -> Histogram<u64> {
        self.catchup
            .lock()
            .map(|h| h.clone())
            .unwrap_or_else(|_| histogram())
    }

    pub fn calls(&self) -> Histogram<u64> {
        self.calls
            .lock()
            .map(|h| h.clone())
            .unwrap_or_else(|_| histogram())
    }
}

impl Stats {
    pub fn new(routes: &[String]) -> Self {
        Self {
            started: Instant::now(),
            routes: routes.iter().map(|name| RouteStats::new(name)).collect(),
            timeline: Mutex::new(Vec::new()),
            restart_ms: AtomicU64::new(0),
            recovered: Mutex::new(Vec::new()),
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    pub fn requested(&self, route: usize) {
        self.routes[route].requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn ok(&self, route: usize, call_ms: Option<u64>) {
        self.routes[route].ok.fetch_add(1, Ordering::Relaxed);
        if let Some(ms) = call_ms {
            record(&self.routes[route].calls, ms);
        }
    }

    pub fn error(&self, route: usize, label: &str) {
        let stats = &self.routes[route];
        stats.errors.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut kinds) = stats.kinds.lock() {
            *kinds.entry(label.to_owned()).or_insert(0) += 1;
        }
    }

    pub fn wake(&self, route: usize, ms: u64) {
        self.routes[route].received.fetch_add(1, Ordering::Relaxed);
        record(&self.routes[route].wake, ms);
    }

    pub fn catchup(&self, route: usize, ms: u64) {
        self.routes[route].received.fetch_add(1, Ordering::Relaxed);
        record(&self.routes[route].catchup, ms);
    }

    pub fn unreadable(&self, route: usize) {
        self.routes[route]
            .unreadable
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn sent(&self, route: usize) {
        self.routes[route].sent.fetch_add(1, Ordering::Relaxed);
    }

    pub fn online(&self, route: usize, delta: i64) {
        self.routes[route]
            .online
            .fetch_add(delta, Ordering::Relaxed);
    }

    pub fn parked(&self, route: usize, delta: i64) {
        self.routes[route]
            .parked
            .fetch_add(delta, Ordering::Relaxed);
    }

    pub fn mark_restart(&self) {
        self.restart_ms
            .store(self.elapsed_ms().max(1), Ordering::Relaxed);
    }

    pub fn restart_ms(&self) -> Option<u64> {
        match self.restart_ms.load(Ordering::Relaxed) {
            0 => None,
            at => Some(at),
        }
    }

    pub fn recovered(&self, at_ms: u64) {
        if let Ok(mut recovered) = self.recovered.lock() {
            recovered.push(at_ms);
        }
    }

    pub fn recovered_list(&self) -> Vec<u64> {
        self.recovered
            .lock()
            .map(|list| list.clone())
            .unwrap_or_default()
    }

    pub fn snapshot(&self) -> Tick {
        let sum = |pick: fn(&RouteStats) -> u64| self.routes.iter().map(pick).sum::<u64>();
        let gauge = |pick: fn(&RouteStats) -> i64| self.routes.iter().map(pick).sum::<i64>();
        Tick {
            t_ms: self.elapsed_ms(),
            online: gauge(|route| route.online.load(Ordering::Relaxed)),
            parked: gauge(|route| route.parked.load(Ordering::Relaxed)),
            ok: sum(|route| route.ok.load(Ordering::Relaxed)),
            errors: sum(|route| route.errors.load(Ordering::Relaxed)),
            sent: sum(|route| route.sent.load(Ordering::Relaxed)),
            received: sum(|route| route.received.load(Ordering::Relaxed)),
        }
    }

    pub fn tick(&self) -> Tick {
        let tick = self.snapshot();
        if let Ok(mut timeline) = self.timeline.lock() {
            timeline.push(tick);
        }
        tick
    }

    pub fn timeline(&self) -> Vec<Tick> {
        self.timeline.lock().map(|t| t.clone()).unwrap_or_default()
    }

    pub fn window(&self, span_ms: u64) -> (u64, u64) {
        let timeline = self.timeline();
        let Some(last) = timeline.last() else {
            return (0, 0);
        };
        let floor = last.t_ms.saturating_sub(span_ms);
        let first = timeline
            .iter()
            .find(|tick| tick.t_ms >= floor)
            .copied()
            .unwrap_or_default();
        (last.ok - first.ok, last.errors - first.errors)
    }

    pub fn wake_all(&self) -> Histogram<u64> {
        let mut all = histogram();
        for route in &self.routes {
            all.add(route.wake()).ok();
        }
        all
    }
}

#[cfg(test)]
#[path = "tests/stats_tests.rs"]
mod tests;
