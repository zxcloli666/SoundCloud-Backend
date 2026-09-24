use std::sync::OnceLock;
use std::time::Duration;

use backend_contracts::JobLane;
use backend_contracts::pipeline::WORKER_STREAMS;
use backend_contracts::reasons::WorkerStatus;
use backend_contracts::worker_contract::{WORKER_LANES, WorkerLane};
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder, PrometheusHandle};
use sqlx::PgPool;

const JOB_DURATION: &str = "jobs_execution_duration_seconds";
const JOB_TOTAL: &str = "jobs_executions_total";
const QUEUE_DEPTH: &str = "jobs_queue_depth";
const QUEUE_OLDEST_DUE: &str = "jobs_queue_oldest_due_seconds";
const QUEUE_DEAD_LETTERS: &str = "jobs_queue_dead_letters";
const POOL_CONNECTIONS: &str = "jobs_pg_pool_connections";
const POOL_WAIT: &str = "jobs_pg_pool_wait_seconds";
const POOL_WAIT_LAST: &str = "jobs_pg_pool_wait_last_seconds";
const COLLAB_SKIPPED: &str = "jobs_collab_skipped_total";

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
            if HANDLE.set(handle).is_ok() {
                register_series_that_start_at_zero();
            }
        }
        Err(error) => tracing::warn!(%error, "metrics recorder unavailable"),
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
    Retryable,
    Terminal,
    Timeout,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
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

pub fn record_worker_lost(lane: WorkerLane, outcome: WorkerLostOutcome) {
    metrics::counter!(WORKER_LOST, "lane" => lane.as_str(), "outcome" => outcome.as_str())
        .increment(1);
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

pub async fn sample_pool_wait(pool: &PgPool) {
    let size = f64::from(pool.size());
    let idle = pool.num_idle() as f64;
    metrics::gauge!(POOL_CONNECTIONS, "state" => "open").set(size);
    metrics::gauge!(POOL_CONNECTIONS, "state" => "idle").set(idle);
    metrics::gauge!(POOL_CONNECTIONS, "state" => "busy").set((size - idle).max(0.0));

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
    metrics::histogram!(POOL_WAIT, "outcome" => outcome).record(waited);
    metrics::gauge!(POOL_WAIT_LAST).set(waited);
}

pub async fn render(pool: &PgPool) -> Option<String> {
    let handle = HANDLE.get()?;
    sample_pool_wait(pool).await;
    let lanes: Vec<String> = [JobLane::CoreFast, JobLane::CoreBulk, JobLane::Ops]
        .iter()
        .map(|lane| lane.as_str().to_owned())
        .collect();
    match sqlx::query_file!("queries/queue/metrics_snapshot.sql", &lanes)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => {
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
    Some(handle.render())
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
