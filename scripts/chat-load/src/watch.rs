use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, bail};
use tokio::process::Command;
use tokio::time::{Instant, sleep};
use tokio_util::sync::CancellationToken;

use crate::prom::{self, Buckets};
use crate::stats::Stats;

pub const ERROR_WINDOW: Duration = Duration::from_secs(60);
pub const ERROR_MIN_REQUESTS: u64 = 200;
pub const RESTART_GRACE: Duration = Duration::from_secs(60);
pub const PROM_WINDOW: Duration = Duration::from_secs(60);
pub const STRIKES_MAX: u32 = 3;
pub const POOL_WAIT_METRIC: &str = "api_pg_pool_wait_seconds";
pub const MUSIC_METRIC: &str = "api_http_request_duration_seconds";

#[derive(Debug, Clone)]
pub struct Check {
    pub name: String,
    pub command: String,
}

pub struct Watcher {
    pub checks: Vec<Check>,
    pub every: Duration,
    pub error_rate: f64,
    pub chat_metrics: Option<String>,
    pub pool_wait_ms: f64,
    pub music_metrics: Option<String>,
    pub music_rise: f64,
    pub music_baseline: Option<f64>,
}

#[derive(Default)]
pub struct Abort {
    reason: Mutex<Option<(u64, String)>>,
    pub token: CancellationToken,
}

impl Abort {
    pub fn trip(&self, at_ms: u64, reason: String) {
        if let Ok(mut held) = self.reason.lock()
            && held.is_none()
        {
            eprintln!("ABORT at {}s: {reason}", at_ms / 1_000);
            *held = Some((at_ms, reason));
        }
        self.token.cancel();
    }

    pub fn reason(&self) -> Option<(u64, String)> {
        self.reason.lock().ok().and_then(|held| held.clone())
    }
}

pub fn named_check(raw: &str) -> anyhow::Result<Check> {
    let (name, command) = raw
        .split_once('=')
        .with_context(|| format!("watch {raw} is not name=command"))?;
    if name.trim().is_empty() || command.trim().is_empty() {
        bail!("watch {raw} is not name=command");
    }
    Ok(Check {
        name: name.trim().to_owned(),
        command: command.to_owned(),
    })
}

pub enum Verdict {
    Fine,
    Crossed(String),
    Blind(String),
}

pub async fn probe(command: &str, timeout: Duration) -> Verdict {
    let child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(timeout, child).await {
        Err(_) => Verdict::Blind("timed out".to_owned()),
        Ok(Err(error)) => Verdict::Blind(error.to_string()),
        Ok(Ok(output)) => {
            let said = String::from_utf8_lossy(&output.stdout);
            let said = said.lines().last().unwrap_or("").trim().to_owned();
            match output.status.code() {
                Some(0) => Verdict::Fine,
                Some(1) => Verdict::Crossed(said),
                code => Verdict::Blind(format!("exit {code:?} {said}")),
            }
        }
    }
}

pub async fn sample(command: &str, metric: &str) -> Option<Buckets> {
    let child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(Duration::from_secs(20), child)
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let found = prom::buckets(&String::from_utf8_lossy(&output.stdout), metric);
    (!found.is_empty()).then_some(found)
}

pub async fn baseline(command: &str, window: Duration) -> Option<f64> {
    let before = sample(command, MUSIC_METRIC).await?;
    sleep(window).await;
    let after = sample(command, MUSIC_METRIC).await?;
    prom::quantile(&before, &after, 0.99)
}

pub fn error_rate_crossed(stats: &Stats, limit: f64) -> Option<String> {
    let now = stats.elapsed_ms();
    if let Some(restart) = stats.restart_ms()
        && now < restart + RESTART_GRACE.as_millis() as u64
    {
        return None;
    }
    let (ok, errors) = stats.window(ERROR_WINDOW.as_millis() as u64);
    let total = ok + errors;
    if total < ERROR_MIN_REQUESTS {
        return None;
    }
    let rate = errors as f64 / total as f64;
    (rate > limit).then(|| {
        format!(
            "harness error rate {:.2}% over the last {} s is above {:.2}%",
            rate * 100.0,
            ERROR_WINDOW.as_secs(),
            limit * 100.0
        )
    })
}

struct Histograms {
    pool: Option<Buckets>,
    music: Option<Buckets>,
    taken: Instant,
}

impl Histograms {
    fn due(&self) -> bool {
        (self.pool.is_none() && self.music.is_none()) || self.taken.elapsed() >= PROM_WINDOW
    }
}

pub async fn run(
    watcher: Arc<Watcher>,
    stats: Arc<Stats>,
    abort: Arc<Abort>,
    stop: CancellationToken,
) {
    let mut strikes = vec![0u32; watcher.checks.len()];
    let measures = watcher.chat_metrics.is_some() || watcher.music_metrics.is_some();
    let mut last = Histograms {
        pool: None,
        music: None,
        taken: Instant::now(),
    };
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            () = sleep(watcher.every) => {}
        }
        if let Some(reason) = error_rate_crossed(&stats, watcher.error_rate) {
            abort.trip(stats.elapsed_ms(), reason);
            return;
        }
        for (at, verdict) in probe_all(&watcher).await {
            let check = &watcher.checks[at];
            match verdict {
                Verdict::Fine => strikes[at] = 0,
                Verdict::Crossed(said) => {
                    abort.trip(stats.elapsed_ms(), format!("{}: {said}", check.name));
                    return;
                }
                Verdict::Blind(why) => {
                    strikes[at] += 1;
                    eprintln!(
                        "watch {} did not answer ({why}), strike {}",
                        check.name, strikes[at]
                    );
                    if strikes[at] >= STRIKES_MAX {
                        abort.trip(
                            stats.elapsed_ms(),
                            format!("{} gave no answer {STRIKES_MAX} times in a row", check.name),
                        );
                        return;
                    }
                }
            }
        }
        if measures
            && last.due()
            && let Some(reason) = histograms_crossed(&watcher, &mut last).await
        {
            abort.trip(stats.elapsed_ms(), reason);
            return;
        }
    }
}

async fn probe_all(watcher: &Watcher) -> Vec<(usize, Verdict)> {
    let mut probes = tokio::task::JoinSet::new();
    for (at, check) in watcher.checks.iter().enumerate() {
        let command = check.command.clone();
        let timeout = watcher.every;
        probes.spawn(async move { (at, probe(&command, timeout).await) });
    }
    let mut verdicts = Vec::with_capacity(watcher.checks.len());
    while let Some(done) = probes.join_next().await {
        if let Ok(verdict) = done {
            verdicts.push(verdict);
        }
    }
    verdicts.sort_by_key(|(at, _)| *at);
    verdicts
}

async fn histograms_crossed(watcher: &Watcher, last: &mut Histograms) -> Option<String> {
    let pool = match &watcher.chat_metrics {
        Some(command) => sample(command, POOL_WAIT_METRIC).await,
        None => None,
    };
    let music = match &watcher.music_metrics {
        Some(command) => sample(command, MUSIC_METRIC).await,
        None => None,
    };
    let mut crossed = None;
    if let (Some(before), Some(after)) = (&last.pool, &pool)
        && let Some(p99) = prom::quantile(before, after, 0.99)
        && p99 * 1_000.0 > watcher.pool_wait_ms
    {
        crossed = Some(format!(
            "chat-api PG pool wait p99 {:.0} ms is above {:.0} ms",
            p99 * 1_000.0,
            watcher.pool_wait_ms
        ));
    }
    if let (Some(before), Some(after), Some(baseline)) =
        (&last.music, &music, watcher.music_baseline)
        && let Some(p99) = prom::quantile(before, after, 0.99)
        && p99 > baseline * (1.0 + watcher.music_rise)
    {
        crossed = crossed.or(Some(format!(
            "music p99 {:.0} ms rose over {:.0}% above the baseline {:.0} ms",
            p99 * 1_000.0,
            watcher.music_rise * 100.0,
            baseline * 1_000.0
        )));
    }
    *last = Histograms {
        pool,
        music,
        taken: Instant::now(),
    };
    crossed
}

#[cfg(test)]
#[path = "tests/watch_tests.rs"]
mod tests;
