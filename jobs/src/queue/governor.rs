use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use backend_contracts::JobKind;
use sqlx::PgPool;
use tokio::sync::Notify;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use super::worker::claim_limit;

const FLOOR: usize = 2;
const CHECK_EVERY: Duration = Duration::from_secs(5);
const RAISE_EVERY: Duration = Duration::from_secs(20);
const LOWER_EVERY: Duration = Duration::from_secs(30);
const MEGABYTE: i64 = 1024 * 1024;
const CALM_BACKLOG_BYTES: i64 = 128 * MEGABYTE;
const HEAVY_BACKLOG_BYTES: i64 = 512 * MEGABYTE;
const CALM_ACTIVE_BACKENDS: i64 = 16;
const HEAVY_ACTIVE_BACKENDS: i64 = 32;
const CALM_WRITE_BYTES_PER_SECOND: f64 = 3.0 * MEGABYTE as f64;
const HEAVY_WRITE_BYTES_PER_SECOND: f64 = 6.0 * MEGABYTE as f64;
const WRITE_RATE_SMOOTHING: f64 = 0.3;
const PRESSURE: &str = include_str!("../../queries/queue/write_pressure.sql");

pub(crate) struct WriteGovernor {
    limit: AtomicUsize,
    ceiling: usize,
    lane: Mutex<LaneUse>,
    freed: Notify,
}

#[derive(Default)]
struct LaneUse {
    running: usize,
    idle: bool,
    due_kinds: Vec<JobKind>,
    lent: usize,
    reserved: usize,
}

impl LaneUse {
    fn held(&self) -> usize {
        self.lent.max(self.reserved)
    }

    fn share(&self, limit: usize, kind: JobKind) -> usize {
        let others = self.due_kinds.iter().filter(|due| **due != kind).count();
        (limit / (others + 1)).max(1)
    }

    fn spare(&self, limit: usize) -> usize {
        if !self.idle {
            return 0;
        }
        limit.saturating_sub(self.running + self.lent + 1)
    }

    fn can_lend(&self, limit: usize) -> bool {
        (self.lent < self.reserved && self.running + self.lent < limit) || self.spare(limit) > 0
    }
}

pub(crate) struct Borrower {
    governor: Arc<WriteGovernor>,
    kind: JobKind,
}

impl Borrower {
    pub(crate) fn slots(&self) -> usize {
        let limit = self.governor.limit();
        let lane = self.governor.lane();
        lane.share(limit, self.kind).max(lane.spare(limit) + 1)
    }

    pub(crate) fn want(&self, slots: usize) {
        let limit = self.governor.limit();
        let mut lane = self.governor.lane();
        lane.reserved = slots.min(lane.share(limit, self.kind)).saturating_sub(1);
    }

    pub(crate) fn lend(&self) -> Option<LentSlot> {
        let limit = self.governor.limit();
        let mut lane = self.governor.lane();
        if !lane.can_lend(limit) {
            return None;
        }
        lane.lent += 1;
        Some(LentSlot {
            governor: self.governor.clone(),
        })
    }

    pub(crate) async fn freed(&self) {
        self.governor.freed.notified().await;
    }
}

impl Drop for Borrower {
    fn drop(&mut self) {
        self.governor.lane().reserved = 0;
    }
}

pub(crate) struct LentSlot {
    governor: Arc<WriteGovernor>,
}

impl Drop for LentSlot {
    fn drop(&mut self) {
        let mut lane = self.governor.lane();
        lane.lent = lane.lent.saturating_sub(1);
    }
}

#[derive(Clone, Copy, Debug, sqlx::FromRow)]
struct Reading {
    replication_backlog_bytes: i64,
    active_backends: i64,
    wal_position: i64,
}

#[derive(Clone, Copy, Debug)]
struct Pressure {
    replication_backlog_bytes: i64,
    active_backends: i64,
    write_bytes_per_second: f64,
}

#[derive(Default)]
struct WriteRate {
    last: Option<(i64, Instant)>,
    smoothed: f64,
}

impl WriteRate {
    fn observe(&mut self, position: i64, now: Instant) -> f64 {
        if let Some((previous, at)) = self.last {
            let seconds = now.saturating_duration_since(at).as_secs_f64();
            if seconds > 0.0 && position >= previous {
                let rate = (position - previous) as f64 / seconds;
                self.smoothed += WRITE_RATE_SMOOTHING * (rate - self.smoothed);
            }
        }
        self.last = Some((position, now));
        self.smoothed
    }
}

impl WriteGovernor {
    pub(crate) fn new(ceiling: usize) -> Arc<Self> {
        Arc::new(Self {
            limit: AtomicUsize::new(FLOOR.min(ceiling)),
            ceiling,
            lane: Mutex::new(LaneUse::default()),
            freed: Notify::new(),
        })
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit.load(Ordering::Relaxed)
    }

    pub(crate) fn claim_room(&self, running: usize, concurrency: usize, batch: usize) -> usize {
        let limit = self.limit();
        let mut lane = self.lane();
        let room = claim_limit(limit.min(concurrency), running + lane.held(), batch);
        lane.running = running + room;
        lane.idle = false;
        self.wake_borrower(&lane, limit);
        room
    }

    pub(crate) fn lane_idle(&self, running: usize) {
        let mut lane = self.lane();
        lane.running = running;
        lane.idle = true;
        self.wake_borrower(&lane, self.limit());
    }

    pub(crate) fn lane_round(&self, due_kinds: &[JobKind]) {
        let mut lane = self.lane();
        lane.due_kinds.clear();
        lane.due_kinds.extend_from_slice(due_kinds);
    }

    pub(crate) fn borrower(self: &Arc<Self>, kind: JobKind) -> Borrower {
        Borrower {
            governor: self.clone(),
            kind,
        }
    }

    #[cfg(test)]
    pub(crate) fn hold_at(&self, limit: usize) {
        self.limit.store(limit, Ordering::Relaxed);
    }

    pub(crate) async fn run(
        self: Arc<Self>,
        pool: PgPool,
        cancellation: CancellationToken,
    ) -> anyhow::Result<()> {
        let mut interval = tokio::time::interval(CHECK_EVERY);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut changed_at = Instant::now();
        let mut write_rate = WriteRate::default();
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => return Ok(()),
                _ = interval.tick() => {}
            }
            match sqlx::query_as::<_, Reading>(PRESSURE)
                .fetch_one(&pool)
                .await
            {
                Ok(reading) => {
                    let pressure = Pressure {
                        replication_backlog_bytes: reading.replication_backlog_bytes,
                        active_backends: reading.active_backends,
                        write_bytes_per_second: write_rate
                            .observe(reading.wal_position, Instant::now()),
                    };
                    if self.adjust(pressure, changed_at.elapsed()) {
                        changed_at = Instant::now();
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "database pressure is unknown, bulk concurrency stays");
                }
            }
        }
    }

    fn adjust(&self, pressure: Pressure, since_change: Duration) -> bool {
        let current = self.limit();
        let next = next_limit(current, self.ceiling, pressure, since_change);
        if next == current {
            return false;
        }
        self.limit.store(next, Ordering::Relaxed);
        self.wake_borrower(&self.lane(), next);
        tracing::info!(
            from = current,
            to = next,
            replication_backlog_mb = pressure.replication_backlog_bytes / MEGABYTE,
            active_backends = pressure.active_backends,
            write_mb_per_second = pressure.write_bytes_per_second / MEGABYTE as f64,
            "bulk concurrency adjusted to the database pressure"
        );
        true
    }

    fn wake_borrower(&self, lane: &LaneUse, limit: usize) {
        if lane.can_lend(limit) {
            self.freed.notify_one();
        }
    }

    fn lane(&self) -> MutexGuard<'_, LaneUse> {
        self.lane.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn next_limit(current: usize, ceiling: usize, pressure: Pressure, since_change: Duration) -> usize {
    let floor = FLOOR.min(ceiling);
    let heavy = pressure.replication_backlog_bytes > HEAVY_BACKLOG_BYTES
        || pressure.active_backends > HEAVY_ACTIVE_BACKENDS
        || pressure.write_bytes_per_second > HEAVY_WRITE_BYTES_PER_SECOND;
    let calm = pressure.replication_backlog_bytes < CALM_BACKLOG_BYTES
        && pressure.active_backends < CALM_ACTIVE_BACKENDS
        && pressure.write_bytes_per_second < CALM_WRITE_BYTES_PER_SECOND;
    if heavy && since_change >= LOWER_EVERY {
        (current / 2).max(floor)
    } else if calm && since_change >= RAISE_EVERY {
        (current + 1).min(ceiling)
    } else {
        current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pressure(backlog_mb: i64, active_backends: i64) -> Pressure {
        Pressure {
            replication_backlog_bytes: backlog_mb * MEGABYTE,
            active_backends,
            write_bytes_per_second: 0.0,
        }
    }

    fn writing(megabytes_per_second: f64) -> Pressure {
        Pressure {
            write_bytes_per_second: megabytes_per_second * MEGABYTE as f64,
            ..pressure(10, 3)
        }
    }

    #[test]
    fn heavy_writing_lowers_the_slots_and_moderate_writing_stops_their_growth() {
        assert_eq!(next_limit(16, 32, writing(9.0), SETTLED), 8);
        assert_eq!(next_limit(16, 32, writing(4.5), SETTLED), 16);
        assert_eq!(next_limit(16, 32, writing(1.0), SETTLED), 17);
    }

    #[test]
    fn the_write_rate_is_smoothed_between_readings() {
        let start = Instant::now();
        let mut rate = WriteRate::default();

        assert_eq!(rate.observe(0, start), 0.0);
        let first = rate.observe(50 * MEGABYTE, start + Duration::from_secs(5));
        let second = rate.observe(100 * MEGABYTE, start + Duration::from_secs(10));

        assert!((first - 3.0 * MEGABYTE as f64).abs() < 1.0);
        assert!(second > first && second < 10.0 * MEGABYTE as f64);
    }

    const SETTLED: Duration = Duration::from_secs(60);

    #[test]
    fn a_calm_database_gets_one_more_slot_up_to_the_ceiling() {
        assert_eq!(next_limit(4, 8, pressure(10, 3), SETTLED), 5);
        assert_eq!(next_limit(8, 8, pressure(10, 3), SETTLED), 8);
    }

    #[test]
    fn a_lagging_replica_or_a_busy_database_halves_the_slots_down_to_the_floor() {
        assert_eq!(next_limit(16, 32, pressure(2000, 3), SETTLED), 8);
        assert_eq!(next_limit(16, 32, pressure(10, 40), SETTLED), 8);
        assert_eq!(next_limit(3, 32, pressure(2000, 3), SETTLED), 2);
        assert_eq!(next_limit(1, 1, pressure(2000, 3), SETTLED), 1);
    }

    #[test]
    fn moderate_pressure_keeps_the_current_slots() {
        assert_eq!(next_limit(10, 32, pressure(300, 3), SETTLED), 10);
        assert_eq!(next_limit(10, 32, pressure(10, 20), SETTLED), 10);
    }

    const FOUR_KINDS: &[JobKind] = &[
        JobKind::CatalogCollection,
        JobKind::IndexTrack,
        JobKind::LyricsLookup,
        JobKind::LyricsLookupSweep,
    ];

    #[test]
    fn a_busy_lane_keeps_the_borrower_its_share_of_the_limit() {
        let governor = WriteGovernor::new(32);
        governor.hold_at(32);
        governor.lane_round(FOUR_KINDS);
        let borrower = governor.borrower(JobKind::LyricsLookupSweep);
        assert_eq!(governor.claim_room(32, 32, 64), 0);
        assert_eq!(borrower.slots(), 8);

        borrower.want(64);
        assert!(borrower.lend().is_none());
        assert_eq!(governor.claim_room(25, 32, 64), 0);
        let lent: Vec<_> = std::iter::from_fn(|| borrower.lend()).collect();

        assert_eq!(lent.len(), 7);
        assert_eq!(governor.claim_room(24, 32, 64), 1);
        governor.lane_round(&FOUR_KINDS[..3]);
        assert_eq!(borrower.slots(), 8);
    }

    #[test]
    fn a_lowered_limit_shrinks_the_share_as_lent_slots_come_back() {
        let governor = WriteGovernor::new(32);
        governor.hold_at(32);
        governor.lane_round(FOUR_KINDS);
        let borrower = governor.borrower(JobKind::LyricsLookupSweep);
        borrower.want(64);
        assert_eq!(governor.claim_room(25, 32, 64), 0);
        let mut lent: Vec<_> = std::iter::from_fn(|| borrower.lend()).collect();
        governor.hold_at(8);
        borrower.want(64);

        assert_eq!(governor.claim_room(6, 32, 64), 0);
        lent.truncate(1);
        assert!(borrower.lend().is_none());
        assert_eq!(governor.claim_room(6, 32, 64), 1);

        lent.clear();
        let last = borrower.lend();
        assert!(last.is_some());
        assert!(borrower.lend().is_none());
    }

    #[test]
    fn an_idle_lane_also_lends_its_spare_slots_but_keeps_one_to_poll() {
        let governor = WriteGovernor::new(32);
        governor.hold_at(8);
        governor.lane_round(FOUR_KINDS);
        let borrower = governor.borrower(JobKind::LyricsLookupSweep);
        borrower.want(64);
        governor.lane_idle(1);
        assert_eq!(borrower.slots(), 7);

        let lent: Vec<_> = std::iter::from_fn(|| borrower.lend()).collect();

        assert_eq!(lent.len(), 6);
        assert_eq!(governor.claim_room(1, 32, 64), 1);
        assert!(borrower.lend().is_none());
    }

    #[test]
    fn a_claim_books_its_room_and_a_finished_borrower_hands_its_share_back() {
        let governor = WriteGovernor::new(32);
        governor.hold_at(8);
        governor.lane_round(FOUR_KINDS);
        let borrower = governor.borrower(JobKind::LyricsLookupSweep);
        borrower.want(64);

        assert_eq!(governor.claim_room(1, 32, 64), 6);
        let lent = borrower.lend();
        assert!(lent.is_some());
        assert!(borrower.lend().is_none());

        drop(lent);
        drop(borrower);
        assert_eq!(governor.claim_room(7, 32, 64), 1);
    }

    #[test]
    fn a_fresh_change_is_given_time_to_show_before_the_next_one() {
        let just_changed = Duration::from_secs(10);

        assert_eq!(next_limit(16, 32, pressure(2000, 3), just_changed), 16);
        assert_eq!(next_limit(4, 32, pressure(10, 3), just_changed), 4);
        assert_eq!(
            next_limit(4, 32, pressure(10, 3), Duration::from_secs(25)),
            5
        );
    }
}
