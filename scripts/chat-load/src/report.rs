use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

use serde::Serialize;

use crate::schedule::{Phase, Schedule};
use crate::stats::{RouteStats, Spread, Stats, Tick, histogram, spread};

pub const WAKE_P99_MAX_MS: u64 = 2_000;
pub const ERROR_RATE_MAX: f64 = 0.01;
pub const RECONNECT_MAX_S: f64 = 60.0;
pub const TIMELINE_STEP_MS: u64 = 5_000;
pub const BEFORE_RESTART_MS: u64 = 30_000;
pub const AFTER_RESTART_MS: u64 = 120_000;
pub const CURVE_POINTS_S: [u64; 10] = [5, 10, 15, 20, 30, 45, 60, 90, 120, 180];

#[derive(Debug, Clone, Serialize)]
pub struct RouteReport {
    pub requests: u64,
    pub ok: u64,
    pub errors: u64,
    pub error_rate: f64,
    pub errors_by_kind: BTreeMap<String, u64>,
    pub sent: u64,
    pub received: u64,
    pub unreadable: u64,
    pub wake: Spread,
    pub catchup: Spread,
    pub calls: Spread,
}

#[derive(Debug, Clone, Serialize)]
pub struct RestartReport {
    pub at_s: f64,
    pub parked_before: f64,
    pub parked_low: i64,
    pub back_95_s: Option<f64>,
    pub online: i64,
    pub recovered: Vec<(u64, f64)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Verdict {
    pub wake_p99_ms: u64,
    pub wake_ok: bool,
    pub error_rate: f64,
    pub errors_ok: bool,
    pub reconnect_s: Option<f64>,
    pub reconnect_ok: bool,
    pub aborted: bool,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub run_id: u32,
    pub duration_s: f64,
    pub schedule: Schedule,
    pub devices_per_route: BTreeMap<String, usize>,
    pub parked_share: f64,
    pub routes: BTreeMap<String, RouteReport>,
    pub total: RouteReport,
    pub restart: Option<RestartReport>,
    pub abort: Option<(f64, String)>,
    pub timeline: Vec<(f64, i64, i64, f64, f64)>,
    pub verdict: Verdict,
}

pub struct Context {
    pub run_id: u32,
    pub schedule: Schedule,
    pub devices_per_route: BTreeMap<String, usize>,
    pub parked_share: f64,
    pub abort: Option<(u64, String)>,
}

fn route_report(route: &RouteStats) -> RouteReport {
    let requests = route.requests.load(Ordering::Relaxed);
    let errors = route.errors.load(Ordering::Relaxed);
    RouteReport {
        requests,
        ok: route.ok.load(Ordering::Relaxed),
        errors,
        error_rate: rate(errors, requests),
        errors_by_kind: route.kinds(),
        sent: route.sent.load(Ordering::Relaxed),
        received: route.received.load(Ordering::Relaxed),
        unreadable: route.unreadable.load(Ordering::Relaxed),
        wake: spread(&route.wake()),
        catchup: spread(&route.catchup()),
        calls: spread(&route.calls()),
    }
}

fn rate(part: u64, whole: u64) -> f64 {
    match whole {
        0 => 0.0,
        whole => part as f64 / whole as f64,
    }
}

fn total(stats: &Stats) -> RouteReport {
    let mut kinds = BTreeMap::new();
    let mut wake = histogram();
    let mut catchup = histogram();
    let mut calls = histogram();
    let sum = |pick: fn(&RouteStats) -> u64| stats.routes.iter().map(pick).sum::<u64>();
    let requests = sum(|r| r.requests.load(Ordering::Relaxed));
    let ok = sum(|r| r.ok.load(Ordering::Relaxed));
    let errors = sum(|r| r.errors.load(Ordering::Relaxed));
    let sent = sum(|r| r.sent.load(Ordering::Relaxed));
    let received = sum(|r| r.received.load(Ordering::Relaxed));
    let unreadable = sum(|r| r.unreadable.load(Ordering::Relaxed));
    for route in &stats.routes {
        for (kind, count) in route.kinds() {
            *kinds.entry(kind).or_insert(0) += count;
        }
        wake.add(route.wake()).ok();
        catchup.add(route.catchup()).ok();
        calls.add(route.calls()).ok();
    }
    RouteReport {
        requests,
        ok,
        errors,
        error_rate: rate(errors, requests),
        errors_by_kind: kinds,
        sent,
        received,
        unreadable,
        wake: spread(&wake),
        catchup: spread(&catchup),
        calls: spread(&calls),
    }
}

pub fn restart_report(timeline: &[Tick], at_ms: u64, recovered: &[u64]) -> RestartReport {
    let before: Vec<&Tick> = timeline
        .iter()
        .filter(|tick| tick.t_ms + BEFORE_RESTART_MS >= at_ms && tick.t_ms < at_ms)
        .collect();
    let parked_before = match before.len() {
        0 => 0.0,
        count => before.iter().map(|tick| tick.parked as f64).sum::<f64>() / count as f64,
    };
    let online = before.last().map_or(0, |tick| tick.online);
    let after: Vec<&Tick> = timeline
        .iter()
        .filter(|tick| tick.t_ms >= at_ms && tick.t_ms <= at_ms + AFTER_RESTART_MS)
        .collect();
    let low_at = after
        .iter()
        .enumerate()
        .min_by_key(|(_, tick)| tick.parked)
        .map_or(0, |(at, _)| at);
    let parked_low = after.get(low_at).map_or(0, |tick| tick.parked);
    let back_95_s = after[low_at.min(after.len())..]
        .iter()
        .find(|tick| tick.parked as f64 >= parked_before * 0.95)
        .map(|tick| (tick.t_ms - at_ms) as f64 / 1_000.0);
    let mut recovered = recovered.to_vec();
    recovered.sort_unstable();
    let curve = CURVE_POINTS_S
        .iter()
        .map(|seconds| {
            let horizon = at_ms + seconds * 1_000;
            let done = recovered.iter().take_while(|at| **at <= horizon).count();
            (*seconds, rate(done as u64, online.max(0) as u64))
        })
        .collect();
    RestartReport {
        at_s: at_ms as f64 / 1_000.0,
        parked_before,
        parked_low,
        back_95_s,
        online,
        recovered: curve,
    }
}

fn timeline_rows(timeline: &[Tick]) -> Vec<(f64, i64, i64, f64, f64)> {
    let mut rows = Vec::new();
    let mut previous: Option<&Tick> = None;
    for tick in timeline {
        if let Some(last) = previous
            && tick.t_ms < last.t_ms + TIMELINE_STEP_MS
        {
            continue;
        }
        let (ok_rate, error_rate) = match previous {
            Some(last) => {
                let seconds = (tick.t_ms - last.t_ms) as f64 / 1_000.0;
                (
                    (tick.ok - last.ok) as f64 / seconds,
                    (tick.errors - last.errors) as f64 / seconds,
                )
            }
            None => (0.0, 0.0),
        };
        rows.push((
            tick.t_ms as f64 / 1_000.0,
            tick.online,
            tick.parked,
            ok_rate,
            error_rate,
        ));
        previous = Some(tick);
    }
    rows
}

pub fn build(stats: &Stats, context: Context) -> Report {
    let timeline = stats.timeline();
    let routes = stats
        .routes
        .iter()
        .map(|route| (route.name.clone(), route_report(route)))
        .collect();
    let total = total(stats);
    let restart = stats
        .restart_ms()
        .map(|at| restart_report(&timeline, at, &stats.recovered_list()));
    let reconnect_s = restart.as_ref().and_then(|restart| restart.back_95_s);
    let reconnect_ok = match &restart {
        None => true,
        Some(_) => reconnect_s.is_some_and(|seconds| seconds <= RECONNECT_MAX_S),
    };
    let wake_ok = total.wake.p99_ms < WAKE_P99_MAX_MS;
    let errors_ok = total.error_rate < ERROR_RATE_MAX;
    let aborted = context.abort.is_some();
    Report {
        run_id: context.run_id,
        duration_s: stats.elapsed_ms() as f64 / 1_000.0,
        schedule: context.schedule,
        devices_per_route: context.devices_per_route,
        parked_share: context.parked_share,
        verdict: Verdict {
            wake_p99_ms: total.wake.p99_ms,
            wake_ok,
            error_rate: total.error_rate,
            errors_ok,
            reconnect_s,
            reconnect_ok,
            aborted,
            passed: wake_ok && errors_ok && reconnect_ok && !aborted,
        },
        routes,
        total,
        restart,
        abort: context
            .abort
            .map(|(at, reason)| (at as f64 / 1_000.0, reason)),
        timeline: timeline_rows(&timeline),
    }
}

pub fn status_line(stats: &Stats, phase: Phase, earlier: Option<Tick>, now: Tick) -> String {
    let (requests, errors) = match earlier {
        Some(earlier) if now.t_ms > earlier.t_ms => {
            let seconds = (now.t_ms - earlier.t_ms) as f64 / 1_000.0;
            let done = (now.ok + now.errors) - (earlier.ok + earlier.errors);
            (
                (done as f64 / seconds),
                rate(now.errors - earlier.errors, done),
            )
        }
        _ => (0.0, 0.0),
    };
    let wake = spread(&stats.wake_all());
    format!(
        "[{:>5}s] {:<8} online {:>6} parked {:>6} req/s {:>7.1} err {:>5.2}% sent {:>7} recv {:>8} wake p50 {} ms p99 {} ms",
        now.t_ms / 1_000,
        phase_name(phase),
        now.online,
        now.parked,
        requests,
        errors * 100.0,
        now.sent,
        now.received,
        wake.p50_ms,
        wake.p99_ms,
    )
}

fn phase_name(phase: Phase) -> String {
    match phase {
        Phase::Ramp(step) => format!("ramp {}", step + 1),
        Phase::Hold => "hold".to_owned(),
        Phase::Worst => "worst".to_owned(),
        Phase::Done => "done".to_owned(),
    }
}

pub fn summary(report: &Report) -> String {
    let mut lines = Vec::new();
    for (name, route) in &report.routes {
        if route.requests == 0 {
            continue;
        }
        lines.push(format!(
            "{name:<8} devices {:>6} requests {:>9} errors {:>7} ({:.2}%) wake p50 {} ms p99 {} ms ({} wakes) catch-up p99 {} ms",
            report.devices_per_route.get(name).copied().unwrap_or(0),
            route.requests,
            route.errors,
            route.error_rate * 100.0,
            route.wake.p50_ms,
            route.wake.p99_ms,
            route.wake.count,
            route.catchup.p99_ms,
        ));
    }
    let verdict = &report.verdict;
    lines.push(format!(
        "verdict: {} (wake p99 {} ms {}, errors {:.2}% {}, reconnect {} {})",
        if verdict.passed { "PASS" } else { "FAIL" },
        verdict.wake_p99_ms,
        mark(verdict.wake_ok),
        verdict.error_rate * 100.0,
        mark(verdict.errors_ok),
        verdict
            .reconnect_s
            .map_or_else(|| "n/a".to_owned(), |s| format!("{s:.0} s")),
        mark(verdict.reconnect_ok),
    ));
    if let Some((at, reason)) = &report.abort {
        lines.push(format!("aborted at {at:.0} s: {reason}"));
    }
    lines.join("\n")
}

fn mark(ok: bool) -> &'static str {
    if ok { "ok" } else { "over" }
}

#[cfg(test)]
#[path = "tests/report_tests.rs"]
mod tests;
