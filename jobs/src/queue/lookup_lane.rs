#[cfg(test)]
#[path = "lookup_lane_tests.rs"]
mod tests;

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use backend_contracts::JobKind;
use tokio::sync::Notify;

use crate::metrics::LookupPath;

use super::governor::WriteGovernor;
use super::model::{ClaimOrder, LeasedJob, QueueError};
use super::repository::JobRepository;

pub(crate) struct LookupLane {
    base: usize,
    governor: Arc<WriteGovernor>,
    held: Mutex<Held>,
    freed: Notify,
}

#[derive(Default)]
struct Held {
    targeted: usize,
    reserved: usize,
    sweep: usize,
    sweep_wants: usize,
    targeted_waiting: bool,
}

impl Held {
    fn targeted_in_flight(&self) -> usize {
        self.targeted.saturating_sub(self.reserved)
    }
}

pub(crate) struct LookupSlot {
    lane: Arc<LookupLane>,
    path: LookupPath,
    reserved: bool,
}

impl Drop for LookupSlot {
    fn drop(&mut self) {
        self.lane.release(self.path, self.reserved);
    }
}

pub(crate) struct SweepDemand {
    lane: Arc<LookupLane>,
}

impl SweepDemand {
    pub(crate) fn wants(&self, lookups: usize) {
        self.lane.held().sweep_wants = lookups;
    }
}

impl Drop for SweepDemand {
    fn drop(&mut self) {
        self.wants(0);
    }
}

impl LookupLane {
    pub(crate) fn new(base: usize, governor: Arc<WriteGovernor>) -> Arc<Self> {
        Arc::new(Self {
            base: base.max(1),
            governor,
            held: Mutex::new(Held::default()),
            freed: Notify::new(),
        })
    }

    pub(crate) fn cap(&self) -> usize {
        let cap = self
            .base
            .saturating_mul(self.governor.limit())
            .div_ceil(self.governor.ceiling().max(1))
            .clamp(1, self.base);
        crate::metrics::record_lyrics_lookup_cap(cap);
        cap
    }

    pub(crate) fn targeted_slots(self: &Arc<Self>) -> Vec<LookupSlot> {
        let cap = self.cap();
        let room = {
            let mut held = self.held();
            let reserve = held
                .sweep_wants
                .min(sweep_share(cap))
                .saturating_sub(held.sweep);
            let room = cap.saturating_sub(held.targeted + held.sweep + reserve);
            held.targeted += room;
            held.reserved += room;
            room
        };
        (0..room)
            .map(|_| self.slot(LookupPath::Targeted, true))
            .collect()
    }

    pub(crate) async fn claim_targeted(
        &self,
        repository: &JobRepository,
        mut slots: Vec<LookupSlot>,
        lease: Duration,
    ) -> Result<Vec<(LeasedJob, LookupSlot)>, QueueError> {
        let reserved = slots.len();
        let jobs = repository
            .claim(
                &[JobKind::LyricsLookup],
                ClaimOrder::Kind(JobKind::LyricsLookup),
                reserved,
                lease,
            )
            .await?;
        slots.truncate(jobs.len());
        let drained = {
            let mut held = self.held();
            held.reserved = held.reserved.saturating_sub(slots.len());
            let waiting = reserved > 0 && jobs.len() == reserved;
            let drained = held.targeted_waiting && !waiting;
            held.targeted_waiting = waiting;
            report(&held);
            drained
        };
        if drained {
            self.freed.notify_one();
        }
        for slot in &mut slots {
            slot.reserved = false;
        }
        Ok(jobs.into_iter().zip(slots).collect())
    }

    pub(crate) fn sweep_slot(self: &Arc<Self>) -> Option<LookupSlot> {
        let cap = self.cap();
        {
            let mut held = self.held();
            if held.sweep >= sweep_limit(cap, held.targeted_waiting)
                || held.targeted + held.sweep >= cap
            {
                return None;
            }
            held.sweep += 1;
            report(&held);
        }
        Some(self.slot(LookupPath::Sweep, false))
    }

    pub(crate) fn sweep_slots(&self) -> usize {
        let cap = self.cap();
        sweep_limit(cap, self.held().targeted_waiting)
    }

    pub(crate) fn sweep_demand(self: &Arc<Self>) -> SweepDemand {
        SweepDemand { lane: self.clone() }
    }

    pub(crate) async fn freed(&self) {
        self.freed.notified().await;
    }

    fn slot(self: &Arc<Self>, path: LookupPath, reserved: bool) -> LookupSlot {
        LookupSlot {
            lane: self.clone(),
            path,
            reserved,
        }
    }

    fn release(&self, path: LookupPath, reserved: bool) {
        {
            let mut held = self.held();
            match path {
                LookupPath::Targeted => held.targeted = held.targeted.saturating_sub(1),
                LookupPath::Sweep => held.sweep = held.sweep.saturating_sub(1),
            }
            if reserved {
                held.reserved = held.reserved.saturating_sub(1);
            }
            report(&held);
        }
        self.freed.notify_one();
    }

    fn held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn sweep_share(cap: usize) -> usize {
    cap.div_ceil(4)
}

fn sweep_limit(cap: usize, targeted_waiting: bool) -> usize {
    if targeted_waiting {
        sweep_share(cap)
    } else {
        cap.saturating_sub(1).max(sweep_share(cap))
    }
}

fn report(held: &Held) {
    crate::metrics::record_lyrics_lookup_slots(held.targeted_in_flight(), held.sweep);
}
