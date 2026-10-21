use std::time::Duration;

use super::{Phase, Schedule};

fn night() -> Schedule {
    Schedule {
        steps: vec![2_000, 5_000, 10_000, 15_000, 20_000],
        step: Duration::from_secs(240),
        spread: Duration::from_secs(60),
        hold: Duration::from_secs(900),
        restart_at: Some(Duration::from_secs(600)),
        worst: Duration::from_secs(300),
    }
}

#[test]
fn the_night_plan_fits_the_hour() {
    let plan = night();
    plan.check(20_000).expect("a valid plan");
    assert_eq!(plan.total(), Duration::from_secs(2_400));
    assert_eq!(plan.restart(), Some(Duration::from_secs(1_800)));
    assert_eq!(plan.worst_starts(), Duration::from_secs(2_100));
}

#[test]
fn devices_join_spread_over_the_start_of_their_step() {
    let plan = night();
    assert_eq!(plan.launch_at(0), Some(Duration::ZERO));
    assert_eq!(plan.launch_at(1_000), Some(Duration::from_secs(30)));
    assert_eq!(plan.launch_at(2_000), Some(Duration::from_secs(240)));
    assert_eq!(plan.launch_at(19_999).map(|at| at.as_secs()), Some(1_019));
    assert_eq!(plan.launch_at(20_000), None);
}

#[test]
fn phases_follow_the_clock() {
    let plan = night();
    assert_eq!(plan.phase(Duration::from_secs(10)), Phase::Ramp(0));
    assert_eq!(plan.phase(Duration::from_secs(1_000)), Phase::Ramp(4));
    assert_eq!(plan.phase(Duration::from_secs(1_300)), Phase::Hold);
    assert_eq!(plan.phase(Duration::from_secs(2_200)), Phase::Worst);
    assert_eq!(plan.phase(Duration::from_secs(2_400)), Phase::Done);
}

#[test]
fn a_broken_plan_is_refused() {
    assert!(night().check(19_999).is_err());
    let falling = Schedule {
        steps: vec![5, 3],
        ..night()
    };
    assert!(falling.check(10).is_err());
    let late_restart = Schedule {
        restart_at: Some(Duration::from_secs(900)),
        ..night()
    };
    assert!(late_restart.check(20_000).is_err());
    let wide = Schedule {
        spread: Duration::from_secs(241),
        ..night()
    };
    assert!(wide.check(20_000).is_err());
}
