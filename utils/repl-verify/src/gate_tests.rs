use std::time::Duration;

use super::gate::{
    CALM_BACKLOG_BYTES, CATCH_UP_GAP, Gate, HOLD_BACKLOG_BYTES, MAX_BEHIND, MAX_HELD_TICKS,
};

const PACE: Duration = Duration::from_secs(5);

#[test]
fn a_lagging_api_node_holds_the_probe_and_the_walk_falls_behind() {
    let mut gate = Gate::default();
    assert!(gate.hold(HOLD_BACKLOG_BYTES + 1, PACE));
    assert!(gate.hold(HOLD_BACKLOG_BYTES + 1, PACE));
    assert_eq!(gate.behind(), PACE * 2);
}

#[test]
fn a_backlog_at_the_hold_line_still_probes() {
    let mut gate = Gate::default();
    assert!(!gate.hold(HOLD_BACKLOG_BYTES, PACE));
    assert_eq!(gate.behind(), Duration::ZERO);
}

#[test]
fn a_long_hold_lets_one_probe_through_and_then_holds_again() {
    let mut gate = Gate::default();
    for _ in 0..MAX_HELD_TICKS {
        assert!(gate.hold(HOLD_BACKLOG_BYTES * 4, PACE));
    }
    assert!(!gate.hold(HOLD_BACKLOG_BYTES * 4, PACE));
    assert_eq!(gate.after_probe(PACE), PACE);
    assert!(gate.hold(HOLD_BACKLOG_BYTES * 4, PACE));
}

#[test]
fn a_calm_node_catches_up_what_the_hold_skipped() {
    let mut gate = Gate::default();
    for _ in 0..3 {
        gate.hold(HOLD_BACKLOG_BYTES + 1, PACE);
    }
    assert!(!gate.hold(CALM_BACKLOG_BYTES - 1, PACE));
    let mut gaps = 0;
    while gate.after_probe(PACE) == CATCH_UP_GAP {
        gaps += 1;
        assert!(!gate.hold(CALM_BACKLOG_BYTES - 1, PACE));
    }
    assert_eq!(gaps, 5);
    assert_eq!(gate.behind(), Duration::ZERO);
}

#[test]
fn a_busy_but_not_lagging_node_keeps_the_normal_pace_and_its_debt() {
    let mut gate = Gate::default();
    gate.hold(HOLD_BACKLOG_BYTES + 1, PACE);
    assert!(!gate.hold(CALM_BACKLOG_BYTES, PACE));
    assert_eq!(gate.after_probe(PACE), PACE);
    assert_eq!(gate.behind(), PACE);
}

#[test]
fn the_debt_stops_growing_at_an_hour() {
    let mut gate = Gate::default();
    for _ in 0..10_000 {
        if !gate.hold(HOLD_BACKLOG_BYTES + 1, PACE) {
            gate.after_probe(PACE);
        }
    }
    assert_eq!(gate.behind(), MAX_BEHIND);
}
