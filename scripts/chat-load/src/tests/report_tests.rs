use super::restart_report;
use crate::stats::Tick;

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
