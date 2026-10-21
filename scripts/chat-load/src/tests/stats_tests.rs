use super::Stats;

#[test]
fn the_window_counts_only_recent_ticks() {
    let stats = Stats::new(&["direct".to_owned(), "r1".to_owned()]);
    stats.requested(0);
    stats.ok(0, Some(12));
    stats.tick();
    stats.error(1, "timeout");
    stats.error(1, "timeout");
    stats.ok(1, None);
    stats.tick();
    let (ok, errors) = stats.window(60_000);
    assert_eq!((ok, errors), (1, 2));
    assert_eq!(stats.routes[1].kinds().get("timeout"), Some(&2));
    assert_eq!(stats.snapshot().ok, 2);
}

#[test]
fn restart_and_recovery_are_kept() {
    let stats = Stats::new(&["direct".to_owned()]);
    assert_eq!(stats.restart_ms(), None);
    stats.mark_restart();
    assert!(stats.restart_ms().is_some());
    stats.recovered(5);
    stats.recovered(7);
    assert_eq!(stats.recovered_list(), [5, 7]);
    stats.wake(0, 40);
    stats.catchup(0, 4_000);
    assert_eq!(stats.wake_all().len(), 1);
    assert_eq!(stats.snapshot().received, 2);
}
