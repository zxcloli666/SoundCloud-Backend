use std::collections::BTreeMap;
use std::time::Duration;

use super::{Context, build, restart_report};
use crate::schedule::Schedule;
use crate::stats::{Stats, Tick};

fn context(abort: Option<(u64, String)>) -> Context {
    Context {
        run_id: 1,
        schedule: Schedule {
            steps: vec![10],
            step: Duration::from_secs(30),
            spread: Duration::from_secs(10),
            hold: Duration::from_secs(60),
            restart_at: None,
            worst: Duration::ZERO,
        },
        devices_per_route: BTreeMap::new(),
        parked_share: 0.3,
        abort,
    }
}

fn tick(t_s: u64, online: i64, parked: i64) -> Tick {
    Tick {
        t_ms: t_s * 1_000,
        online,
        parked,
        ..Tick::default()
    }
}

#[test]
fn the_reconnect_curve_measures_the_dip_and_the_way_back() {
    let mut timeline: Vec<Tick> = (0..30).map(|s| tick(70 + s, 1_000, 300)).collect();
    timeline.extend([
        tick(100, 1_000, 280),
        tick(101, 1_000, 20),
        tick(105, 1_000, 0),
        tick(110, 1_000, 150),
        tick(118, 1_000, 290),
        tick(130, 1_000, 300),
    ]);
    let recovered: Vec<u64> = (0..1_000u64).map(|n| 100_000 + n * 20).collect();
    let report = restart_report(&timeline, 100_000, &recovered);
    assert_eq!(report.parked_before, 300.0);
    assert_eq!(report.parked_low, 0);
    assert_eq!(report.back_95_s, Some(18.0));
    assert_eq!(report.online, 1_000);
    assert_eq!(report.recovered[0], (5, 0.251));
    assert_eq!(report.recovered[4], (30, 1.0));
}

#[test]
fn a_run_that_sent_messages_and_delivered_none_does_not_pass() {
    let stats = Stats::new(&["direct".to_owned()]);
    for _ in 0..10 {
        stats.requested(0);
        stats.ok(0, Some(5));
    }
    stats.sent(0);
    stats.tick();
    let silent = build(&stats, context(None));
    assert!(!silent.verdict.delivered_ok);
    assert!(!silent.verdict.passed);

    stats.wake(0, 80);
    let heard = build(&stats, context(None));
    assert!(heard.verdict.delivered_ok);
    assert!(heard.verdict.passed);

    let aborted = build(&stats, context(Some((5_000, "relay cpu".to_owned()))));
    assert!(!aborted.verdict.passed);
}
