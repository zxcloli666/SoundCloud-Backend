use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::Context;
use async_nats::{Message, Subscriber};
use backend_contracts::pipeline::{DONE_STREAM, WORKER_STATUS_WILDCARD};
use backend_contracts::reasons::WorkerStatus;
use backend_contracts::worker_contract::WorkerLane;
use futures::StreamExt;
use serde::Deserialize;
use tokio::task::{JoinError, JoinSet};
use tokio_util::sync::CancellationToken;

use crate::bus::{Bus, WorkerQueueSnapshot};
use crate::metrics::NodeSeries;

pub const STATUS_FRESH_FOR: Duration = Duration::from_secs(60);
pub const DONE_FILL_LIMIT: f64 = 0.8;
const WORKER_INVALID_WILDCARD: &str = "worker.invalid.>";
const STATUS_SUBJECT_PREFIX: &str = "worker.status.";
const INVALID_SUBJECT_PREFIX: &str = "worker.invalid.";
const QUEUE_POLL_INTERVAL: Duration = Duration::from_secs(30);
const GAUGE_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const RESUBSCRIBE_DELAY: Duration = Duration::from_secs(5);
const RESTART_WINDOW: Duration = Duration::from_secs(60 * 60);
const FRESH_WORKER_UPTIME_S: f64 = 30.0;
const MAX_NODES: usize = 256;
const MAX_SLOTS_PER_NODE: usize = 16;
const NODE_FORGOTTEN_AFTER: Duration = RESTART_WINDOW;
const PUBLIC_TRUST: &str = "public";
const PUBLIC_NODE_PREFIX: &str = "public.";
const SERVING: &str = "serving";
const ABSENT: &str = "absent";
const QUEUE_POLL_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone)]
pub struct WorkerStatusBoard {
    board: Arc<Mutex<Board>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkerHealth {
    pub stale_lanes: Vec<&'static str>,
    pub done_fill_ratio: Option<f64>,
}

impl WorkerHealth {
    pub fn is_healthy(&self) -> bool {
        self.stale_lanes.is_empty()
            && self
                .done_fill_ratio
                .is_none_or(|ratio| ratio <= DONE_FILL_LIMIT)
    }
}

struct Board {
    listening_since: Instant,
    required: Vec<WorkerLane>,
    nodes: BTreeMap<String, Node>,
    done_fill_ratio: Option<f64>,
}

struct Node {
    seen: Instant,
    trusted: bool,
    lanes: BTreeMap<&'static str, LaneView>,
    slots: BTreeMap<String, SlotView>,
}

#[derive(Default)]
struct LaneView {
    state: String,
    last_serving: Option<Instant>,
    done: BTreeMap<&'static str, u64>,
}

#[derive(Default)]
struct SlotView {
    restarts: VecDeque<(Instant, u64)>,
    reserved_gap_mib: f64,
}

#[derive(Debug, Deserialize)]
struct StatusReport {
    #[serde(default)]
    trust: Option<String>,
    #[serde(default)]
    uptime_s: f64,
    #[serde(default)]
    lanes: BTreeMap<String, LaneReport>,
    #[serde(default)]
    slots: BTreeMap<String, SlotReport>,
}

#[derive(Debug, Deserialize)]
struct LaneReport {
    #[serde(default)]
    state: String,
    #[serde(default)]
    done: BTreeMap<String, u64>,
}

#[derive(Debug, Deserialize)]
struct SlotReport {
    #[serde(default)]
    restarts: u64,
    #[serde(default)]
    reserved_gap_mib: f64,
}

impl WorkerStatusBoard {
    pub fn new() -> Self {
        Self {
            board: Arc::new(Mutex::new(Board {
                listening_since: Instant::now(),
                required: Vec::new(),
                nodes: BTreeMap::new(),
                done_fill_ratio: None,
            })),
        }
    }

    pub fn require_lanes(&self, lanes: &[WorkerLane]) {
        for lane in WorkerLane::ALL {
            crate::metrics::record_worker_lane_required(lane, lanes.contains(&lane));
        }
        self.lock().required = lanes.to_vec();
    }

    pub fn record_status(&self, node: &str, payload: &[u8], now: Instant) -> anyhow::Result<()> {
        let report: StatusReport =
            serde_json::from_slice(payload).context("worker status is not readable")?;
        let mut board = self.lock();
        let known = board.nodes.contains_key(node);
        let entry = board.nodes.entry(node.to_owned()).or_insert_with(|| Node {
            seen: now,
            trusted: true,
            lanes: BTreeMap::new(),
            slots: BTreeMap::new(),
        });
        let counts_from_zero = !known && report.uptime_s <= FRESH_WORKER_UPTIME_S;
        entry.seen = now;
        entry.trusted =
            !node.starts_with(PUBLIC_NODE_PREFIX) && report.trust.as_deref() != Some(PUBLIC_TRUST);
        absorb_lanes(entry, &report.lanes, known, counts_from_zero, now);
        absorb_slots(entry, &report.slots, now);
        forget_silent(&mut board.nodes, now);
        evict_oldest(&mut board.nodes);
        Ok(())
    }

    pub fn record_queue(&self, snapshot: &WorkerQueueSnapshot) {
        for backlog in &snapshot.consumers {
            crate::metrics::record_worker_consumer_backlog(
                backlog.durable,
                backlog.pending,
                backlog.waiting,
            );
        }
        for fill in &snapshot.streams {
            crate::metrics::record_worker_stream_fill(fill.stream, fill.ratio);
        }
        if let Some(done) = snapshot
            .streams
            .iter()
            .find(|fill| fill.stream == DONE_STREAM.name)
        {
            self.lock().done_fill_ratio = Some(done.ratio);
        }
    }

    pub fn refresh(&self, now: Instant) {
        let mut board = self.lock();
        forget_silent(&mut board.nodes, now);
        for lane in WorkerLane::ALL {
            crate::metrics::record_worker_lane_fleet(
                lane,
                board.lane_status_age(lane, now),
                board.serving_nodes(lane, now),
            );
        }
        let slots = board.slot_fleet(now);
        crate::metrics::record_worker_slot_fleet(
            slots.restarts_last_hour_max,
            slots.reserved_gap_mib_max,
        );
    }

    pub fn node_series(&self, now: Instant) -> NodeSeries {
        let board = self.lock();
        let mut series = NodeSeries::default();
        for (name, node) in &board.nodes {
            series.status_age(name, now.saturating_duration_since(node.seen));
            for (lane_name, view) in &node.lanes {
                let Some(lane) = lane_named(lane_name) else {
                    continue;
                };
                series.lane_state(name, lane, &view.state);
                for (status, total) in &view.done {
                    series.done(name, lane, status, *total);
                }
            }
            for (slot, view) in &node.slots {
                let restarts = view.restarts.back().map_or(0, |(_, count)| *count);
                series.slot(name, slot, restarts, view.reserved_gap_mib);
            }
        }
        series
    }

    pub fn health(&self, now: Instant) -> WorkerHealth {
        let board = self.lock();
        WorkerHealth {
            stale_lanes: board
                .required
                .iter()
                .filter(|lane| board.lane_status_age(**lane, now) >= STATUS_FRESH_FOR)
                .map(|lane| lane.as_str())
                .collect(),
            done_fill_ratio: board.done_fill_ratio,
        }
    }

    pub async fn run(self, bus: Bus, cancellation: CancellationToken) -> anyhow::Result<()> {
        let queues = bus.clone();
        self.run_with(bus, cancellation, move || {
            let queues = queues.clone();
            async move { queues.worker_queue_snapshot().await }
        })
        .await
    }

    pub(crate) async fn run_with<P, F>(
        self,
        bus: Bus,
        cancellation: CancellationToken,
        poll_queue: P,
    ) -> anyhow::Result<()>
    where
        P: Fn() -> F,
        F: Future<Output = WorkerQueueSnapshot> + Send + 'static,
    {
        while !cancellation.is_cancelled() {
            match self.listen(&bus, &cancellation, &poll_queue).await {
                Ok(()) => return Ok(()),
                Err(error) => {
                    tracing::warn!(
                        error = format!("{error:#}"),
                        "worker status listener restarts"
                    );
                    tokio::select! {
                        _ = cancellation.cancelled() => return Ok(()),
                        _ = tokio::time::sleep(RESUBSCRIBE_DELAY) => {}
                    }
                }
            }
        }
        Ok(())
    }

    async fn listen<P, F>(
        &self,
        bus: &Bus,
        cancellation: &CancellationToken,
        poll_queue: &P,
    ) -> anyhow::Result<()>
    where
        P: Fn() -> F,
        F: Future<Output = WorkerQueueSnapshot> + Send + 'static,
    {
        let mut statuses: Subscriber = bus.subscribe(WORKER_STATUS_WILDCARD).await?;
        let mut invalid: Subscriber = bus.subscribe(WORKER_INVALID_WILDCARD).await?;
        let mut poll = tokio::time::interval(QUEUE_POLL_INTERVAL);
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut refresh = tokio::time::interval(GAUGE_REFRESH_INTERVAL);
        refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut snapshots = JoinSet::new();

        loop {
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => return Ok(()),
                message = statuses.next() => {
                    let message = message.context("worker status subscription closed")?;
                    self.accept_status(&message);
                }
                message = invalid.next() => {
                    let message = message.context("worker invalid subscription closed")?;
                    accept_invalid(&message);
                }
                Some(finished) = snapshots.join_next(), if !snapshots.is_empty() => {
                    self.accept_queue(finished);
                }
                _ = poll.tick(), if snapshots.is_empty() => {
                    let snapshot = poll_queue();
                    snapshots.spawn(async move {
                        tokio::time::timeout(QUEUE_POLL_TIMEOUT, snapshot).await.ok()
                    });
                }
                _ = refresh.tick() => self.refresh(Instant::now()),
            }
        }
    }

    fn accept_queue(&self, finished: Result<Option<WorkerQueueSnapshot>, JoinError>) {
        match finished {
            Ok(Some(snapshot)) => self.record_queue(&snapshot),
            Ok(None) => tracing::warn!(
                timeout_s = QUEUE_POLL_TIMEOUT.as_secs(),
                "worker queue snapshot timed out"
            ),
            Err(error) => tracing::warn!(%error, "worker queue snapshot failed"),
        }
    }

    fn accept_status(&self, message: &Message) {
        let Some(node) = message.subject.strip_prefix(STATUS_SUBJECT_PREFIX) else {
            return;
        };
        if let Err(error) = self.record_status(node, &message.payload, Instant::now()) {
            tracing::warn!(node, error = format!("{error:#}"), "worker status ignored");
        }
    }

    fn lock(&self) -> MutexGuard<'_, Board> {
        self.board
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Default for WorkerStatusBoard {
    fn default() -> Self {
        Self::new()
    }
}

impl Board {
    fn lane_status_age(&self, lane: WorkerLane, now: Instant) -> Duration {
        let newest = self
            .nodes
            .values()
            .filter(|node| node.trusted)
            .filter_map(|node| {
                node.lanes
                    .get(lane.as_str())
                    .and_then(|view| view.last_serving)
            })
            .max()
            .unwrap_or(self.listening_since);
        now.saturating_duration_since(newest)
    }

    fn serving_nodes(&self, lane: WorkerLane, now: Instant) -> usize {
        self.nodes
            .values()
            .filter(|node| now.saturating_duration_since(node.seen) < STATUS_FRESH_FOR)
            .filter(|node| {
                node.lanes
                    .get(lane.as_str())
                    .is_some_and(|view| view.state == SERVING)
            })
            .count()
    }

    fn slot_fleet(&self, now: Instant) -> SlotFleet {
        let mut fleet = SlotFleet::default();
        for node in self.nodes.values() {
            let fresh = now.saturating_duration_since(node.seen) < STATUS_FRESH_FOR;
            for slot in node.slots.values() {
                fleet.restarts_last_hour_max = fleet
                    .restarts_last_hour_max
                    .max(slot.restarts_in_window(now));
                if fresh {
                    fleet.reserved_gap_mib_max =
                        fleet.reserved_gap_mib_max.max(slot.reserved_gap_mib);
                }
            }
        }
        fleet
    }
}

#[derive(Debug, Default, PartialEq)]
struct SlotFleet {
    restarts_last_hour_max: u64,
    reserved_gap_mib_max: f64,
}

impl SlotView {
    fn restarts_in_window(&self, now: Instant) -> u64 {
        let Some((_, last)) = self.restarts.back() else {
            return 0;
        };
        let baseline = self
            .restarts
            .iter()
            .rev()
            .find(|(at, _)| now.saturating_duration_since(*at) >= RESTART_WINDOW)
            .or(self.restarts.front())
            .map_or(*last, |(_, count)| *count);
        if *last >= baseline {
            last - baseline
        } else {
            *last
        }
    }
}

fn absorb_lanes(
    node: &mut Node,
    reports: &BTreeMap<String, LaneReport>,
    known: bool,
    counts_from_zero: bool,
    now: Instant,
) {
    for (name, view) in &mut node.lanes {
        if !reports.contains_key(*name) {
            view.state = ABSENT.to_owned();
        }
    }
    for (name, report) in reports {
        let Some(lane) = lane_named(name) else {
            continue;
        };
        let view = node.lanes.entry(lane.as_str()).or_default();
        view.state.clone_from(&report.state);
        if report.state == SERVING {
            view.last_serving = Some(now);
        }
        for status in WorkerStatus::ALL {
            let total = report.done.get(status.as_str()).copied().unwrap_or(0);
            let previous = view.done.insert(status.as_str(), total);
            let delta = done_since(previous, total, known || counts_from_zero);
            crate::metrics::record_worker_lane_done(lane, status, delta);
        }
    }
}

fn lane_named(name: &str) -> Option<WorkerLane> {
    WorkerLane::ALL
        .into_iter()
        .find(|lane| lane.as_str() == name)
}

fn done_since(previous: Option<u64>, total: u64, counts_from_zero: bool) -> u64 {
    match previous {
        Some(previous) if total >= previous => total - previous,
        Some(_) => total,
        None if counts_from_zero => total,
        None => 0,
    }
}

fn absorb_slots(node: &mut Node, reports: &BTreeMap<String, SlotReport>, now: Instant) {
    for (name, report) in reports {
        if !node.slots.contains_key(name) && node.slots.len() >= MAX_SLOTS_PER_NODE {
            continue;
        }
        let view = node.slots.entry(name.clone()).or_default();
        view.restarts.push_back((now, report.restarts));
        while view
            .restarts
            .get(1)
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) >= RESTART_WINDOW)
        {
            view.restarts.pop_front();
        }
        view.reserved_gap_mib = report.reserved_gap_mib;
    }
}

fn forget_silent(nodes: &mut BTreeMap<String, Node>, now: Instant) {
    nodes.retain(|_, node| now.saturating_duration_since(node.seen) < NODE_FORGOTTEN_AFTER);
}

fn evict_oldest(nodes: &mut BTreeMap<String, Node>) {
    while nodes.len() > MAX_NODES {
        let Some(oldest) = nodes
            .iter()
            .min_by_key(|(_, node)| node.seen)
            .map(|(name, _)| name.clone())
        else {
            return;
        };
        nodes.remove(&oldest);
    }
}

fn accept_invalid(message: &Message) {
    let lane = message
        .subject
        .strip_prefix(INVALID_SUBJECT_PREFIX)
        .unwrap_or_default();
    tracing::warn!(
        lane,
        bytes = message.payload.len(),
        "worker could not read a task and set it aside"
    );
    crate::metrics::record_worker_invalid(lane);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn status(trust: &str, lanes: serde_json::Value) -> Vec<u8> {
        json!({ "trust": trust, "uptime_s": 600.0, "lanes": lanes })
            .to_string()
            .into_bytes()
    }

    fn serving(lane: &str) -> serde_json::Value {
        json!({ lane: { "state": "serving", "done": { "ok": 3 } } })
    }

    #[test]
    fn a_required_lane_is_healthy_only_while_a_trusted_node_keeps_serving_it() -> anyhow::Result<()>
    {
        let board = WorkerStatusBoard::new();
        board.require_lanes(&[WorkerLane::Audio]);
        let start = Instant::now();

        board.record_status("gpu-main", &status("trusted", serving("audio")), start)?;

        assert!(board.health(start + Duration::from_secs(59)).is_healthy());
        let late = board.health(start + STATUS_FRESH_FOR);
        assert_eq!(late.stale_lanes, vec!["audio"]);
        assert!(!late.is_healthy());
        Ok(())
    }

    #[test]
    fn a_public_node_does_not_stand_in_for_the_trusted_host() -> anyhow::Result<()> {
        let board = WorkerStatusBoard::new();
        board.require_lanes(&[WorkerLane::Transcribe]);
        let start = Instant::now();

        board.record_status(
            "public.node-7",
            &status("public", serving("transcribe")),
            start,
        )?;

        assert_eq!(
            board.health(start + STATUS_FRESH_FOR).stale_lanes,
            vec!["transcribe"]
        );
        Ok(())
    }

    #[test]
    fn a_node_bridged_from_the_public_broker_never_counts_as_trusted_whatever_it_claims()
    -> anyhow::Result<()> {
        let board = WorkerStatusBoard::new();
        board.require_lanes(&[WorkerLane::Transcribe]);
        let start = Instant::now();
        let unlabelled = json!({ "uptime_s": 600.0, "lanes": serving("transcribe") })
            .to_string()
            .into_bytes();

        board.record_status(
            "public.node-7",
            &status("trusted", serving("transcribe")),
            start,
        )?;
        board.record_status("public.node-8", &unlabelled, start)?;

        assert_eq!(
            board.health(start + STATUS_FRESH_FOR).stale_lanes,
            vec!["transcribe"]
        );
        board.record_status("gpu-main", &unlabelled, start + STATUS_FRESH_FOR)?;
        assert!(board.health(start + STATUS_FRESH_FOR).is_healthy());
        Ok(())
    }

    #[test]
    fn a_lane_dropped_from_a_report_stops_counting_as_served_by_that_node() -> anyhow::Result<()> {
        let board = WorkerStatusBoard::new();
        let start = Instant::now();
        let later = start + Duration::from_secs(15);

        board.record_status("gpu-main", &status("trusted", serving("audio")), start)?;
        assert_eq!(board.lock().serving_nodes(WorkerLane::Audio, start), 1);
        board.record_status("gpu-main", &status("trusted", serving("lyrics")), later)?;

        let board = board.lock();
        assert_eq!(board.serving_nodes(WorkerLane::Audio, later), 0);
        assert_eq!(board.serving_nodes(WorkerLane::Lyrics, later), 1);
        let audio = board
            .nodes
            .get("gpu-main")
            .and_then(|node| node.lanes.get("audio"))
            .context("the dropped lane keeps its history")?;
        assert_eq!(audio.state, ABSENT);
        assert_eq!(audio.last_serving, Some(start));
        Ok(())
    }

    #[test]
    fn a_lane_that_stopped_serving_goes_stale_even_while_its_node_keeps_reporting()
    -> anyhow::Result<()> {
        let board = WorkerStatusBoard::new();
        board.require_lanes(&[WorkerLane::Lyrics]);
        let start = Instant::now();
        let degraded = json!({ "lyrics": { "state": "degraded" } });

        board.record_status("gpu-main", &status("trusted", serving("lyrics")), start)?;
        board.record_status(
            "gpu-main",
            &status("trusted", degraded),
            start + Duration::from_secs(45),
        )?;

        assert!(!board.health(start + STATUS_FRESH_FOR).is_healthy());
        Ok(())
    }

    #[test]
    fn a_full_result_stream_makes_the_worker_unhealthy() {
        let board = WorkerStatusBoard::new();
        let snapshot = |ratio| WorkerQueueSnapshot {
            consumers: Vec::new(),
            streams: vec![crate::bus::worker_consumers::StreamFill {
                stream: DONE_STREAM.name,
                ratio,
            }],
        };

        board.record_queue(&snapshot(0.5));
        assert!(board.health(Instant::now()).is_healthy());
        board.record_queue(&snapshot(0.9));
        assert!(!board.health(Instant::now()).is_healthy());
    }

    #[test]
    fn nothing_is_required_until_the_deployment_says_so() {
        let board = WorkerStatusBoard::new();

        assert!(
            board
                .health(Instant::now() + Duration::from_secs(3600))
                .is_healthy()
        );
    }

    #[test]
    fn slot_restarts_count_only_the_last_hour_and_survive_a_worker_restart() -> anyhow::Result<()> {
        let board = WorkerStatusBoard::new();
        let start = Instant::now();
        let restarts = |count: u64| {
            json!({ "trust": "trusted", "slots": { "sep": { "restarts": count } } })
                .to_string()
                .into_bytes()
        };
        let minutes = |value: u64| start + Duration::from_secs(value * 60);

        for (at, count) in [(0, 0), (10, 2), (30, 5), (70, 5)] {
            board.record_status("gpu-main", &restarts(count), minutes(at))?;
        }
        let in_window = |board: &WorkerStatusBoard, now: Instant| {
            board
                .lock()
                .nodes
                .get("gpu-main")
                .and_then(|node| node.slots.get("sep"))
                .map(|slot| slot.restarts_in_window(now))
        };
        assert_eq!(in_window(&board, minutes(70)), Some(3));

        board.record_status("gpu-main", &restarts(1), minutes(75))?;
        assert_eq!(in_window(&board, minutes(75)), Some(1));
        Ok(())
    }

    fn slots(restarts: u64, reserved_gap_mib: f64) -> Vec<u8> {
        json!({
            "trust": "trusted",
            "uptime_s": 600.0,
            "lanes": serving("audio"),
            "slots": { "sep": { "restarts": restarts, "reserved_gap_mib": reserved_gap_mib } },
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn a_node_that_fell_silent_stops_holding_the_fleet_slot_alarms() -> anyhow::Result<()> {
        let board = WorkerStatusBoard::new();
        let start = Instant::now();
        let minutes = |value: u64| start + Duration::from_secs(value * 60);

        board.record_status("gpu-main", &slots(0, 0.0), minutes(0))?;
        board.record_status("gpu-main", &slots(5, 2048.0), minutes(10))?;
        let fleet = |now| board.lock().slot_fleet(now);

        assert_eq!(
            fleet(minutes(10)),
            SlotFleet {
                restarts_last_hour_max: 5,
                reserved_gap_mib_max: 2048.0
            }
        );
        assert_eq!(
            fleet(minutes(12)),
            SlotFleet {
                restarts_last_hour_max: 5,
                reserved_gap_mib_max: 0.0
            }
        );
        assert_eq!(fleet(minutes(70)), SlotFleet::default());
        Ok(())
    }

    #[test]
    fn a_node_silent_for_an_hour_leaves_the_board_and_the_exported_series() -> anyhow::Result<()> {
        let board = WorkerStatusBoard::new();
        let start = Instant::now();
        board.record_status("public.node-7", &slots(2, 64.0), start)?;
        board.record_status("gpu-main", &slots(0, 0.0), start)?;

        let early = board.node_series(start + Duration::from_secs(5)).render();
        assert!(early.contains("jobs_worker_status_age_seconds{node=\"public.node-7\"} 5"));
        assert!(early.contains(
            "jobs_worker_lane_state{lane=\"audio\",node=\"public.node-7\",state=\"serving\"} 1"
        ));
        assert!(early.contains(
            "jobs_worker_done_total{lane=\"audio\",node=\"public.node-7\",status=\"ok\"} 3"
        ));
        assert!(
            early
                .contains("jobs_worker_slot_restarts_total{node=\"public.node-7\",slot=\"sep\"} 2")
        );

        board.record_status("gpu-main", &slots(0, 0.0), start + NODE_FORGOTTEN_AFTER)?;
        board.refresh(start + NODE_FORGOTTEN_AFTER);
        let late = board.node_series(start + NODE_FORGOTTEN_AFTER).render();
        assert!(!late.contains("public.node-7"));
        assert!(late.contains("node=\"gpu-main\""));
        assert_eq!(board.lock().nodes.len(), 1);
        Ok(())
    }

    #[test]
    fn a_node_evicted_from_a_full_board_takes_its_series_with_it() -> anyhow::Result<()> {
        let board = WorkerStatusBoard::new();
        let start = Instant::now();
        for index in 0..=MAX_NODES {
            let at = start + Duration::from_secs(u64::try_from(index)?);
            board.record_status(&format!("public.n{index}"), &slots(0, 0.0), at)?;
        }

        let series = board.node_series(start + Duration::from_secs(300)).render();
        assert!(!series.contains("node=\"public.n0\""));
        assert!(series.contains(&format!("node=\"public.n{MAX_NODES}\"")));
        Ok(())
    }

    #[test]
    fn node_and_slot_names_chosen_by_a_worker_cannot_break_the_exposition() -> anyhow::Result<()> {
        let board = WorkerStatusBoard::new();
        let start = Instant::now();
        let many_slots: serde_json::Map<String, serde_json::Value> = (0..100)
            .map(|index| (format!("s{index:03}"), json!({ "restarts": 1 })))
            .chain([("a\"b\\c".to_owned(), json!({ "restarts": 1 }))])
            .collect();
        let report = json!({ "trust": "public", "slots": many_slots })
            .to_string()
            .into_bytes();

        board.record_status("public.q\"x", &report, start)?;

        let series = board.node_series(start).render();
        assert!(series.contains("node=\"public.q\\\"x\""));
        assert!(!series.contains("node=\"public.q\"x\""));
        let node = board.lock();
        let slots = node
            .nodes
            .get("public.q\"x")
            .map(|node| node.slots.len())
            .context("the node is on the board")?;
        assert_eq!(slots, MAX_SLOTS_PER_NODE);
        Ok(())
    }

    #[test]
    fn done_counts_turn_into_increments_across_worker_restarts() {
        assert_eq!(done_since(Some(10), 14, false), 4);
        assert_eq!(done_since(Some(10), 3, false), 3);
        assert_eq!(done_since(None, 50, false), 0);
        assert_eq!(done_since(None, 2, true), 2);
    }

    #[test]
    fn the_board_forgets_the_longest_silent_node_first() -> anyhow::Result<()> {
        let board = WorkerStatusBoard::new();
        let start = Instant::now();
        for index in 0..=MAX_NODES {
            let at = start + Duration::from_secs(u64::try_from(index)?);
            board.record_status(&format!("node-{index}"), &status("public", json!({})), at)?;
        }

        let nodes = &board.lock().nodes;
        assert_eq!(nodes.len(), MAX_NODES);
        assert!(!nodes.contains_key("node-0"));
        Ok(())
    }

    #[test]
    fn a_status_that_is_not_json_is_refused_without_touching_the_board() {
        let board = WorkerStatusBoard::new();

        assert!(
            board
                .record_status("gpu-main", b"not json", Instant::now())
                .is_err()
        );
        assert!(board.lock().nodes.is_empty());
    }
}
