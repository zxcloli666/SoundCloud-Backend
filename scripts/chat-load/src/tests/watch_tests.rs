use std::time::Duration;

use super::{Verdict, error_rate_crossed, named_check, probe};
use crate::stats::Stats;

#[tokio::test]
async fn a_check_aborts_on_exit_one_and_is_blind_otherwise() {
    let second = Duration::from_secs(5);
    assert!(matches!(probe("exit 0", second).await, Verdict::Fine));
    match probe("echo relay cpu 81%; exit 1", second).await {
        Verdict::Crossed(said) => assert_eq!(said, "relay cpu 81%"),
        _ => panic!("exit 1 crosses"),
    }
    assert!(matches!(probe("exit 2", second).await, Verdict::Blind(_)));
    assert!(matches!(
        probe("sleep 3", Duration::from_millis(100)).await,
        Verdict::Blind(_)
    ));
}

#[test]
fn checks_are_named() {
    let check = named_check("r1-cpu=checks/relay-cpu.sh sc-relay1 75").expect("a check");
    assert_eq!(check.name, "r1-cpu");
    assert_eq!(check.command, "checks/relay-cpu.sh sc-relay1 75");
    assert!(named_check("nothing").is_err());
    assert!(named_check("=x").is_err());
}

#[test]
fn the_error_rate_needs_enough_requests() {
    let stats = Stats::new(&["direct".to_owned()]);
    for _ in 0..500 {
        stats.ok(0, None);
    }
    for _ in 0..20 {
        stats.error(0, "timeout");
    }
    stats.tick();
    assert_eq!(error_rate_crossed(&stats, 0.01), None);
    for _ in 0..1_000 {
        stats.ok(0, None);
    }
    for _ in 0..30 {
        stats.error(0, "connect");
    }
    stats.tick();
    assert!(error_rate_crossed(&stats, 0.01).is_some());
    stats.mark_restart();
    assert_eq!(error_rate_crossed(&stats, 0.01), None);
}
