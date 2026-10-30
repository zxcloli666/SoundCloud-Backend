use std::sync::OnceLock;
use std::time::{Duration, Instant};

use backend_contracts::pipeline::WORKER_STREAMS;
use backend_contracts::reasons::WorkerStatus;
use backend_contracts::worker_contract::{WORKER_LANES, WorkerLane};
use backend_contracts::{JobKind, JobLane};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use sqlx::PgPool;

const JOB_DURATION: &str = "jobs_execution_duration_seconds";
const JOB_TOTAL: &str = "jobs_executions_total";
const QUEUE_DEPTH: &str = "jobs_queue_depth";
const QUEUE_OLDEST_DUE: &str = "jobs_queue_oldest_due_seconds";
const QUEUE_DEAD_LETTERS: &str = "jobs_queue_dead_letters";
const QUEUE_KIND_DEPTH: &str = "jobs_queue_kind_depth";
const QUEUE_KIND_OLDEST_DUE: &str = "jobs_queue_kind_oldest_due_seconds";
const QUEUE_KIND_OLDEST_AGE: &str = "jobs_queue_kind_oldest_age_seconds";
const POOL_CONNECTIONS: &str = "jobs_pg_pool_connections";
const POOL_WAIT: &str = "jobs_pg_pool_wait_seconds";
const POOL_WAIT_LAST: &str = "jobs_pg_pool_wait_last_seconds";
const COLLAB_SKIPPED: &str = "jobs_collab_skipped_total";
const STAR_PRIORITY_PICKED: &str = "jobs_star_priority_picked_total";
const STAR_PRIORITY_WAITING: &str = "jobs_star_priority_waiting_seconds";
const STAR_PRIORITY_QUEUED_WAITING: &str = "jobs_star_priority_queued_waiting_seconds";

const CMD_BACKLOG: &str = "scd_cmd_backlog";
const CMD_AGE: &str = "scd_cmd_age_seconds";
const CMD_SUB_UP: &str = "scd_cmd_sub_up";
const CMD_DEAD: &str = "scd_cmd_dead";
const CMD_DEAD_OLDEST: &str = "scd_cmd_dead_oldest_seconds";
const CMD_REPLAYED: &str = "scd_cmd_replayed_total";
const RETENTION_DELETED: &str = "scd_repl_retention_deleted_total";
const RETENTION_HELD: &str = "scd_repl_retention_held";

const WORKER_STATUS_AGE: &str = "jobs_worker_status_age_seconds";
const WORKER_LANE_STATE: &str = "jobs_worker_lane_state";
const WORKER_DONE: &str = "jobs_worker_done_total";
const WORKER_SLOT_RESTARTS: &str = "jobs_worker_slot_restarts_total";
const WORKER_SLOT_RESERVED_GAP: &str = "jobs_worker_slot_reserved_gap_mib";
const WORKER_LANE_STATUS_AGE: &str = "jobs_worker_lane_status_age_seconds";
const WORKER_LANE_REQUIRED: &str = "jobs_worker_lane_required";
const WORKER_LANE_SERVING_NODES: &str = "jobs_worker_lane_serving_nodes";
const WORKER_LANE_DONE: &str = "jobs_worker_lane_done_total";
const WORKER_SLOT_RESTARTS_LAST_HOUR_MAX: &str = "jobs_worker_slot_restarts_last_hour_max";
const WORKER_SLOT_RESERVED_GAP_MAX: &str = "jobs_worker_slot_reserved_gap_mib_max";
const WORKER_CONSUMER_PENDING: &str = "jobs_worker_consumer_pending";
const WORKER_CONSUMER_WAITING: &str = "jobs_worker_consumer_waiting";
const WORKER_CONSUMER_RECREATED: &str = "jobs_worker_consumer_recreated_total";
const WORKER_STREAM_FILL: &str = "jobs_worker_stream_fill_ratio";
const WORKER_LOST: &str = "jobs_worker_lost_total";
const WORKER_INVALID: &str = "jobs_worker_invalid_total";
const WORKER_RESULT_TIMEOUTS: &str = "jobs_worker_result_timeouts_total";
const WORKER_RESULT_TIMEOUTS_REPEATED: &str = "jobs_worker_result_timeouts_repeated_total";

pub const WORKER_LANE_STATES: [&str; 6] = [
    "serving",
    "not_provisioned",
    "not_served",
    "degraded",
    "paused",
    "draining",
];
const UNKNOWN_LANE: &str = "unknown";

const POOL_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const POOL_WAIT_BUCKETS: &[f64] = &[
    0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 2.5, 5.0,
];
const DURATION_BUCKETS: &[f64] = &[0.01, 0.05, 0.1, 0.5, 1.0, 5.0, 15.0, 60.0, 300.0, 600.0];

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();
static LANE_SNAPSHOT_AT: tokio::sync::Mutex<Option<Instant>> = tokio::sync::Mutex::const_new(None);
static KIND_SNAPSHOT_AT: tokio::sync::Mutex<Option<Instant>> = tokio::sync::Mutex::const_new(None);
static REPL_SNAPSHOT_AT: tokio::sync::Mutex<Option<Instant>> = tokio::sync::Mutex::const_new(None);
const SNAPSHOT_TTL: Duration = Duration::from_secs(60);
const REPL_SNAPSHOT_TTL: Duration = Duration::from_secs(15);
const RETENTION_TABLES: [&str; 4] = ["cmd_done", "cmd_dead", "repl_purges", "repl_tombstones"];
const REPLAY_RESULTS: [&str; 3] = ["ok", "missing", "failed"];
const UPKEEP_EVERY: Duration = Duration::from_secs(5);

pub fn init() {
    if HANDLE.get().is_some() {
        return;
    }
    let builder = PrometheusBuilder::new()
        .set_buckets_for_metric(Matcher::Full(JOB_DURATION.to_owned()), DURATION_BUCKETS)
        .and_then(|builder| {
            builder.set_buckets_for_metric(Matcher::Full(POOL_WAIT.to_owned()), POOL_WAIT_BUCKETS)
        });
    let builder = match builder {
        Ok(builder) => builder,
        Err(error) => {
            tracing::warn!(%error, "metrics buckets rejected");
            return;
        }
    };
    match builder.install_recorder() {
        Ok(handle) => {
            register_series_that_start_at_zero();
            if HANDLE.set(handle.clone()).is_ok() {
                start_upkeep(handle);
            }
        }
        Err(error) => tracing::warn!(%error, "metrics recorder unavailable"),
    }
}

fn start_upkeep(handle: PrometheusHandle) {
    let started = std::thread::Builder::new()
        .name("metrics-upkeep".to_owned())
        .spawn(move || {
            loop {
                std::thread::sleep(UPKEEP_EVERY);
                handle.run_upkeep();
            }
        });
    if let Err(error) = started {
        tracing::warn!(%error, "metrics upkeep did not start");
    }
}

fn register_series_that_start_at_zero() {
    for reason in CollabSkipReason::ALL {
        metrics::counter!(COLLAB_SKIPPED, "reason" => reason.as_str()).increment(0);
    }
    for spec in &WORKER_LANES {
        let lane = spec.lane.as_str();
        metrics::gauge!(WORKER_LANE_STATUS_AGE, "lane" => lane).set(0.0);
        metrics::gauge!(WORKER_LANE_REQUIRED, "lane" => lane).set(0.0);
        metrics::gauge!(WORKER_LANE_SERVING_NODES, "lane" => lane).set(0.0);
        metrics::gauge!(WORKER_CONSUMER_PENDING, "durable" => spec.durable).set(0.0);
        metrics::gauge!(WORKER_CONSUMER_WAITING, "durable" => spec.durable).set(0.0);
        metrics::counter!(WORKER_CONSUMER_RECREATED, "lane" => lane).increment(0);
        metrics::counter!(WORKER_INVALID, "lane" => lane).increment(0);
        metrics::counter!(WORKER_RESULT_TIMEOUTS, "lane" => lane).increment(0);
        metrics::counter!(WORKER_RESULT_TIMEOUTS_REPEATED, "lane" => lane).increment(0);
        for status in WorkerStatus::ALL {
            metrics::counter!(WORKER_LANE_DONE, "lane" => lane, "status" => status.as_str())
                .increment(0);
        }
        for outcome in WorkerLostOutcome::ALL {
            metrics::counter!(WORKER_LOST, "lane" => lane, "outcome" => outcome.as_str())
                .increment(0);
        }
    }
    for stream in &WORKER_STREAMS {
        metrics::gauge!(WORKER_STREAM_FILL, "stream" => stream.name).set(0.0);
    }
    metrics::gauge!(WORKER_SLOT_RESTARTS_LAST_HOUR_MAX).set(0.0);
    metrics::gauge!(WORKER_SLOT_RESERVED_GAP_MAX).set(0.0);
    for table in RETENTION_TABLES {
        metrics::counter!(RETENTION_DELETED, "table" => table).increment(0);
        metrics::gauge!(RETENTION_HELD, "table" => table).set(0.0);
    }
    for result in REPLAY_RESULTS {
        metrics::counter!(CMD_REPLAYED, "result" => result).increment(0);
    }
}

pub fn record_repl_retention(table: &'static str, deleted: u64, held: bool) {
    metrics::counter!(RETENTION_DELETED, "table" => table).increment(deleted);
    metrics::gauge!(RETENTION_HELD, "table" => table).set(f64::from(u8::from(held)));
}

pub fn record_repl_replay(result: &'static str) {
    metrics::counter!(CMD_REPLAYED, "result" => result).increment(1);
}

pub fn record_execution(kind: &'static str, outcome: Outcome, elapsed: Duration) {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::histogram!(JOB_DURATION, "kind" => kind).record(elapsed.as_secs_f64());
    metrics::counter!(JOB_TOTAL, "kind" => kind, "outcome" => outcome.as_str()).increment(1);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Postponed,
    Retryable,
    Terminal,
    Timeout,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Postponed => "postponed",
            Self::Retryable => "retryable",
            Self::Terminal => "terminal",
            Self::Timeout => "timeout",
        }
    }
}

pub fn record_collab_skipped(reason: CollabSkipReason) {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::counter!(COLLAB_SKIPPED, "reason" => reason.as_str()).increment(1);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollabSkipReason {
    TooFewSessions,
    EmptyVocab,
    BelowBaseline,
    InFlight,
}

impl CollabSkipReason {
    pub const ALL: [Self; 4] = [
        Self::TooFewSessions,
        Self::EmptyVocab,
        Self::BelowBaseline,
        Self::InFlight,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TooFewSessions => "too_few_sessions",
            Self::EmptyVocab => "empty_vocab",
            Self::BelowBaseline => "below_baseline",
            Self::InFlight => "in_flight",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerLostOutcome {
    Applied,
    Settled,
    Dropped,
    Failed,
}

impl WorkerLostOutcome {
    pub const ALL: [Self; 4] = [Self::Applied, Self::Settled, Self::Dropped, Self::Failed];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::Settled => "settled",
            Self::Dropped => "dropped",
            Self::Failed => "failed",
        }
    }
}

pub fn record_star_priority_round(
    pipeline: &'static str,
    tier: &'static str,
    picked: usize,
    waiting_seconds: i64,
) {
    metrics::counter!(STAR_PRIORITY_PICKED, "pipeline" => pipeline, "tier" => tier)
        .increment(u64::try_from(picked).unwrap_or(u64::MAX));
    metrics::gauge!(STAR_PRIORITY_WAITING, "pipeline" => pipeline, "tier" => tier)
        .set(waiting_seconds as f64);
}

pub fn record_star_priority_queued(kind: &'static str, tier: &'static str, waiting_seconds: f64) {
    metrics::gauge!(STAR_PRIORITY_QUEUED_WAITING, "kind" => kind, "tier" => tier)
        .set(waiting_seconds);
}

pub fn record_worker_lost(lane: WorkerLane, outcome: WorkerLostOutcome) {
    metrics::counter!(WORKER_LOST, "lane" => lane.as_str(), "outcome" => outcome.as_str())
        .increment(1);
}

pub fn record_worker_result_timeouts(lane: WorkerLane, timed_out: u64, repeated: u64) {
    metrics::counter!(WORKER_RESULT_TIMEOUTS, "lane" => lane.as_str()).increment(timed_out);
    metrics::counter!(WORKER_RESULT_TIMEOUTS_REPEATED, "lane" => lane.as_str()).increment(repeated);
}

pub fn record_worker_consumer_recreated(lane: WorkerLane) {
    metrics::counter!(WORKER_CONSUMER_RECREATED, "lane" => lane.as_str()).increment(1);
}

pub fn record_worker_invalid(lane: &str) {
    let lane = WorkerLane::ALL
        .into_iter()
        .map(WorkerLane::as_str)
        .find(|known| *known == lane)
        .unwrap_or(UNKNOWN_LANE);
    metrics::counter!(WORKER_INVALID, "lane" => lane).increment(1);
}

pub fn record_worker_consumer_backlog(durable: &'static str, pending: u64, waiting: usize) {
    metrics::gauge!(WORKER_CONSUMER_PENDING, "durable" => durable).set(pending as f64);
    metrics::gauge!(WORKER_CONSUMER_WAITING, "durable" => durable).set(waiting as f64);
}

pub fn record_worker_stream_fill(stream: &'static str, ratio: f64) {
    metrics::gauge!(WORKER_STREAM_FILL, "stream" => stream).set(ratio);
}

pub fn record_worker_lane_required(lane: WorkerLane, required: bool) {
    metrics::gauge!(WORKER_LANE_REQUIRED, "lane" => lane.as_str())
        .set(f64::from(u8::from(required)));
}

pub fn record_worker_lane_fleet(lane: WorkerLane, status_age: Duration, serving_nodes: usize) {
    metrics::gauge!(WORKER_LANE_STATUS_AGE, "lane" => lane.as_str()).set(status_age.as_secs_f64());
    metrics::gauge!(WORKER_LANE_SERVING_NODES, "lane" => lane.as_str()).set(serving_nodes as f64);
}

pub fn record_worker_lane_done(lane: WorkerLane, status: WorkerStatus, delta: u64) {
    metrics::counter!(WORKER_LANE_DONE, "lane" => lane.as_str(), "status" => status.as_str())
        .increment(delta);
}

pub fn record_worker_slot_fleet(restarts_last_hour_max: u64, reserved_gap_mib_max: f64) {
    metrics::gauge!(WORKER_SLOT_RESTARTS_LAST_HOUR_MAX).set(restarts_last_hour_max as f64);
    metrics::gauge!(WORKER_SLOT_RESERVED_GAP_MAX).set(reserved_gap_mib_max);
}

#[derive(Default)]
pub struct NodeSeries {
    status_age: String,
    lane_state: String,
    done: String,
    slot_restarts: String,
    slot_reserved_gap: String,
}

impl NodeSeries {
    pub fn status_age(&mut self, node: &str, age: Duration) {
        self.status_age.push_str(&format!(
            "{WORKER_STATUS_AGE}{{node=\"{}\"}} {}\n",
            label(node),
            age.as_secs_f64()
        ));
    }

    pub fn lane_state(&mut self, node: &str, lane: WorkerLane, state: &str) {
        for known in WORKER_LANE_STATES {
            self.lane_state.push_str(&format!(
                "{WORKER_LANE_STATE}{{lane=\"{}\",node=\"{}\",state=\"{known}\"}} {}\n",
                lane.as_str(),
                label(node),
                u8::from(known == state)
            ));
        }
    }

    pub fn done(&mut self, node: &str, lane: WorkerLane, status: &str, total: u64) {
        self.done.push_str(&format!(
            "{WORKER_DONE}{{lane=\"{}\",node=\"{}\",status=\"{}\"}} {total}\n",
            lane.as_str(),
            label(node),
            label(status)
        ));
    }

    pub fn slot(&mut self, node: &str, slot: &str, restarts: u64, reserved_gap_mib: f64) {
        self.slot_restarts.push_str(&format!(
            "{WORKER_SLOT_RESTARTS}{{node=\"{}\",slot=\"{}\"}} {restarts}\n",
            label(node),
            label(slot)
        ));
        self.slot_reserved_gap.push_str(&format!(
            "{WORKER_SLOT_RESERVED_GAP}{{node=\"{}\",slot=\"{}\"}} {reserved_gap_mib}\n",
            label(node),
            label(slot)
        ));
    }

    pub fn render(self) -> String {
        [
            (WORKER_STATUS_AGE, "gauge", self.status_age),
            (WORKER_LANE_STATE, "gauge", self.lane_state),
            (WORKER_DONE, "counter", self.done),
            (WORKER_SLOT_RESTARTS, "counter", self.slot_restarts),
            (WORKER_SLOT_RESERVED_GAP, "gauge", self.slot_reserved_gap),
        ]
        .into_iter()
        .filter(|(_, _, samples)| !samples.is_empty())
        .map(|(name, kind, samples)| format!("# TYPE {name} {kind}\n{samples}"))
        .collect()
    }
}

pub fn with_node_series(mut body: String, nodes: NodeSeries) -> String {
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(&nodes.render());
    body
}

fn label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

#[derive(Clone)]
pub struct MeteredPool {
    pub name: &'static str,
    pub pool: PgPool,
}

pub async fn sample_pool_wait(metered: &MeteredPool) {
    let MeteredPool { name, pool } = metered;
    let size = f64::from(pool.size());
    let idle = pool.num_idle() as f64;
    metrics::gauge!(POOL_CONNECTIONS, "pool" => *name, "state" => "open").set(size);
    metrics::gauge!(POOL_CONNECTIONS, "pool" => *name, "state" => "idle").set(idle);
    metrics::gauge!(POOL_CONNECTIONS, "pool" => *name, "state" => "busy")
        .set((size - idle).max(0.0));

    let started = std::time::Instant::now();
    let outcome: &'static str = match tokio::time::timeout(POOL_PROBE_TIMEOUT, pool.acquire()).await
    {
        Ok(Ok(connection)) => {
            drop(connection);
            "ok"
        }
        Ok(Err(_)) => "error",
        Err(_) => "timeout",
    };
    let waited = started.elapsed().as_secs_f64();
    metrics::histogram!(POOL_WAIT, "pool" => *name, "outcome" => outcome).record(waited);
    metrics::gauge!(POOL_WAIT_LAST, "pool" => *name).set(waited);
}

pub async fn render(pool: &PgPool, metered: &[MeteredPool]) -> Option<String> {
    let handle = HANDLE.get()?;
    futures::future::join_all(metered.iter().map(sample_pool_wait)).await;
    record_lane_depths(pool).await;
    record_kind_depths(pool).await;
    record_repl_ops(pool).await;
    Some(handle.render())
}

#[cfg(test)]
pub(crate) async fn forget_snapshots() {
    *LANE_SNAPSHOT_AT.lock().await = None;
    *KIND_SNAPSHOT_AT.lock().await = None;
    *REPL_SNAPSHOT_AT.lock().await = None;
}

async fn record_lane_depths(pool: &PgPool) {
    let mut snapshot_at = LANE_SNAPSHOT_AT.lock().await;
    if snapshot_at.is_some_and(|taken| taken.elapsed() < SNAPSHOT_TTL) {
        return;
    }
    let lanes: Vec<String> = JobLane::ALL
        .iter()
        .map(|lane| lane.as_str().to_owned())
        .collect();
    match sqlx::query_file!("queries/queue/metrics_snapshot.sql", &lanes)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => {
            *snapshot_at = Some(Instant::now());
            for row in rows {
                let lane = row.lane;
                metrics::gauge!(QUEUE_DEPTH, "lane" => lane.clone(), "state" => "pending")
                    .set(row.pending as f64);
                metrics::gauge!(QUEUE_DEPTH, "lane" => lane.clone(), "state" => "due")
                    .set(row.due as f64);
                metrics::gauge!(QUEUE_DEPTH, "lane" => lane.clone(), "state" => "leased")
                    .set(row.leased as f64);
                metrics::gauge!(QUEUE_DEPTH, "lane" => lane.clone(), "state" => "lease_expired")
                    .set(row.expired as f64);
                metrics::gauge!(QUEUE_DEPTH, "lane" => lane.clone(), "state" => "retried")
                    .set(row.retried as f64);
                metrics::gauge!(QUEUE_OLDEST_DUE, "lane" => lane.clone())
                    .set(row.oldest_due_seconds);
                metrics::gauge!(QUEUE_DEAD_LETTERS, "lane" => lane).set(row.dead_letters as f64);
            }
        }
        Err(error) => tracing::warn!(%error, "queue metrics snapshot failed"),
    }
}

async fn record_kind_depths(pool: &PgPool) {
    let mut snapshot_at = KIND_SNAPSHOT_AT.lock().await;
    if snapshot_at.is_some_and(|taken| taken.elapsed() < SNAPSHOT_TTL) {
        return;
    }
    let rows = match sqlx::query_file!("queries/queue/metrics_kinds.sql")
        .fetch_all(pool)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%error, "queue kind metrics snapshot failed");
            return;
        }
    };
    *snapshot_at = Some(Instant::now());
    for kind in JobKind::ALL {
        let row = rows.iter().find(|row| row.kind == kind.as_str());
        let lane = kind.lane().as_str();
        let name = kind.as_str();
        let (pending, due, leased, oldest_due, oldest_age) =
            row.map_or((0, 0, 0, 0.0, 0.0), |row| {
                (
                    row.pending,
                    row.due,
                    row.leased,
                    row.oldest_due_seconds,
                    row.oldest_age_seconds,
                )
            });
        metrics::gauge!(QUEUE_KIND_DEPTH, "lane" => lane, "kind" => name, "state" => "pending")
            .set(pending as f64);
        metrics::gauge!(QUEUE_KIND_DEPTH, "lane" => lane, "kind" => name, "state" => "due")
            .set(due as f64);
        metrics::gauge!(QUEUE_KIND_DEPTH, "lane" => lane, "kind" => name, "state" => "leased")
            .set(leased as f64);
        metrics::gauge!(QUEUE_KIND_OLDEST_DUE, "lane" => lane, "kind" => name).set(oldest_due);
        metrics::gauge!(QUEUE_KIND_OLDEST_AGE, "lane" => lane, "kind" => name).set(oldest_age);
    }
    if let Err(error) = crate::handlers::star_priority::record_queued_waits(pool).await {
        tracing::warn!(%error, "star priority queue metrics snapshot failed");
    }
}

async fn record_repl_ops(pool: &PgPool) {
    let mut snapshot_at = REPL_SNAPSHOT_AT.lock().await;
    if snapshot_at.is_some_and(|taken| taken.elapsed() < REPL_SNAPSHOT_TTL) {
        return;
    }
    let (subscriptions, dead) = tokio::join!(
        sqlx::query_file!("queries/repl/metrics_subscriptions.sql").fetch_all(pool),
        sqlx::query_file!("queries/repl/metrics_dead.sql").fetch_all(pool)
    );
    match (subscriptions, dead) {
        (Ok(subscriptions), Ok(dead)) => {
            *snapshot_at = Some(Instant::now());
            for row in subscriptions {
                let sub = row.sub;
                metrics::gauge!(CMD_SUB_UP, "sub" => sub.clone()).set(f64::from(u8::from(row.up)));
                metrics::gauge!(CMD_BACKLOG, "sub" => sub.clone()).set(row.backlog_bytes);
                metrics::gauge!(CMD_AGE, "sub" => sub).set(row.age_seconds);
            }
            for row in dead {
                let kind = row.kind;
                metrics::gauge!(CMD_DEAD, "kind" => kind.clone()).set(row.dead as f64);
                metrics::gauge!(CMD_DEAD_OLDEST, "kind" => kind).set(row.oldest_seconds);
            }
        }
        (Err(error), _) | (_, Err(error)) => {
            tracing::warn!(%error, "replication metrics snapshot failed");
        }
    }
}

#[cfg(test)]
fn rendered() -> Option<String> {
    HANDLE.get().map(PrometheusHandle::render)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_contract_dimension_reads_zero_before_any_worker_reports() {
        init();
        let Some(body) = rendered() else {
            return;
        };

        for spec in &WORKER_LANES {
            for series in [
                format!(
                    "jobs_worker_consumer_pending{{durable=\"{}\"}} 0",
                    spec.durable
                ),
                format!(
                    "jobs_worker_consumer_waiting{{durable=\"{}\"}} 0",
                    spec.durable
                ),
                format!(
                    "jobs_worker_consumer_recreated_total{{lane=\"{}\"}}",
                    spec.lane.as_str()
                ),
                format!(
                    "jobs_worker_lane_required{{lane=\"{}\"}}",
                    spec.lane.as_str()
                ),
                format!(
                    "jobs_worker_lane_status_age_seconds{{lane=\"{}\"}}",
                    spec.lane.as_str()
                ),
                format!(
                    "jobs_worker_lost_total{{lane=\"{}\",outcome=\"applied\"}}",
                    spec.lane.as_str()
                ),
            ] {
                assert!(body.contains(&series), "{series} is not exported at start");
            }
        }
        for stream in &WORKER_STREAMS {
            let series = format!(
                "jobs_worker_stream_fill_ratio{{stream=\"{}\"}}",
                stream.name
            );
            assert!(body.contains(&series), "{series} is not exported at start");
        }
        for reason in CollabSkipReason::ALL {
            let series = format!(
                "jobs_collab_skipped_total{{reason=\"{}\"}}",
                reason.as_str()
            );
            assert!(body.contains(&series), "{series} is not exported at start");
        }
    }

    #[test]
    fn an_invalid_message_from_a_lane_outside_the_contract_does_not_grow_the_label_set() {
        init();
        record_worker_invalid("made-up-lane");
        let Some(body) = rendered() else {
            return;
        };

        assert!(body.contains("jobs_worker_invalid_total{lane=\"unknown\"}"));
        assert!(!body.contains("made-up-lane"));
    }
}
