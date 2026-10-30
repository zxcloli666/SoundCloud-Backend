use std::convert::Infallible;
use std::str::FromStr;
use std::time::Duration;

use sqlx::PgPool;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

pub const LAST_ATTEMPT: i32 = 3;

const BUDGET: i32 = 10;
const CHUNK: i32 = 500;
const ANSWERS: i32 = 50;
const PACE: Duration = Duration::from_secs(5);
const IDLE_PACE: Duration = Duration::from_secs(60);
const MAX_BACKOFF: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Dry,
    Repair,
}

impl FromStr for Mode {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "off" => Ok(Mode::Off),
            "dry" => Ok(Mode::Dry),
            "repair" => Ok(Mode::Repair),
            _ => Err("expected off, dry or repair".to_owned()),
        }
    }
}

#[derive(Debug)]
pub struct Probe {
    pub state: String,
    pub probes: i32,
    pub rel: Option<String>,
    pub wrapped: bool,
}

#[derive(Debug)]
pub struct Repair {
    pub peer: String,
    pub rel: String,
    pub k: Option<String>,
    pub action: String,
    pub attempt: i32,
}

impl Repair {
    pub fn failed(&self) -> bool {
        self.action.starts_with("error")
    }

    pub fn unresolved(&self) -> bool {
        self.attempt >= LAST_ATTEMPT
    }
}

pub async fn probe_once(pool: &PgPool, budget: i32, chunk: i32) -> Result<Probe, sqlx::Error> {
    sqlx::query_file_as!(Probe, "queries/probe.sql", budget, chunk)
        .fetch_one(pool)
        .await
}

pub async fn repair_once(
    pool: &PgPool,
    dry: bool,
    answers: i32,
) -> Result<Vec<Repair>, sqlx::Error> {
    sqlx::query_file_as!(Repair, "queries/repair.sql", dry, answers)
        .fetch_all(pool)
        .await
}

pub struct Verifier {
    pool: PgPool,
    mode: Mode,
    pace: Duration,
}

impl Verifier {
    pub fn new(pool: PgPool, mode: Mode) -> Self {
        Self {
            pool,
            mode,
            pace: PACE,
        }
    }

    pub fn with_pace(self, pace: Duration) -> Self {
        Self { pace, ..self }
    }

    pub async fn run(self, stop: CancellationToken) -> Result<(), Infallible> {
        let mut backoff = self.pace;
        loop {
            let wait = match self.step().await {
                Ok(true) => {
                    backoff = self.pace;
                    self.pace
                }
                Ok(false) => IDLE_PACE,
                Err(error) => {
                    warn!(%error, "scd verify step failed");
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                    backoff
                }
            };
            tokio::select! {
                () = stop.cancelled() => return Ok(()),
                () = tokio::time::sleep(wait) => {}
            }
        }
    }

    async fn step(&self) -> Result<bool, sqlx::Error> {
        if self.mode == Mode::Off {
            return Ok(false);
        }
        let probe = probe_once(&self.pool, BUDGET, CHUNK).await?;
        if probe.state == "idle" {
            return Ok(false);
        }
        metrics::counter!("scd_repl_verify_probes_total").increment(probe.probes as u64);
        if probe.wrapped {
            metrics::counter!("scd_repl_verify_cycles_total").increment(1);
            info!("scd verify probed every table once");
        }
        let dry = self.mode == Mode::Dry;
        for repair in repair_once(&self.pool, dry, ANSWERS).await? {
            report(&repair, dry);
        }
        Ok(true)
    }
}

fn report(repair: &Repair, dry: bool) {
    let key = repair.k.as_deref().unwrap_or("-");
    let action = if repair.failed() {
        "error"
    } else {
        repair.action.as_str()
    };
    metrics::counter!(
        "scd_repl_verify_repairs_total",
        "peer" => repair.peer.clone(),
        "tbl" => repair.rel.clone(),
        "action" => action.to_owned(),
        "dry" => dry.to_string()
    )
    .increment(1);
    if repair.failed() {
        warn!(peer = %repair.peer, tbl = %repair.rel, key, action = %repair.action, "scd verify could not settle an answer");
        return;
    }
    if repair.unresolved() {
        metrics::counter!("scd_repl_verify_unresolved_total", "peer" => repair.peer.clone(), "tbl" => repair.rel.clone())
            .increment(1);
        warn!(peer = %repair.peer, tbl = %repair.rel, key, action, attempt = repair.attempt, dry, "scd verify: the key still differs after the last attempt");
        return;
    }
    info!(peer = %repair.peer, tbl = %repair.rel, key, action, attempt = repair.attempt, dry, "scd verify repair");
}
