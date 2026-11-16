use super::*;

const FIRST: &str = "{\"say\":\"first\"}";
const SPARE: &str = "{\"say\":\"spare\"}";

fn limited((provider, probe): (Provider, Probe), per_minute: u32) -> (Provider, Probe) {
    (provider.limited_to(per_minute), probe)
}

fn results_of(bench: &Bench, provider: &str) -> Vec<&'static str> {
    bench
        .seen
        .steps()
        .into_iter()
        .filter(|(id, _)| id == provider)
        .map(|(_, result)| result)
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_step_over_its_rate_is_skipped_and_the_next_one_answers() {
    let (first, first_probe) = limited(free("first", vec![text(10, FIRST)]), 2);
    let (spare, spare_probe) = free("spare", vec![text(10, SPARE)]);
    let bench = uncapped(vec![first, spare]);

    for _ in 0..2 {
        assert_eq!(answer(&bench.ladder).await.0, FIRST);
    }
    assert_eq!(answer(&bench.ladder).await.0, SPARE);
    assert_eq!((first_probe.started(), spare_probe.started()), (2, 1));
    assert_eq!(results_of(&bench, "first"), ["won", "won", "skipped_rate"]);
    assert!(!bench.ladder.steps[0].breaker.is_open(Instant::now()));

    tokio::time::advance(Duration::from_secs(60)).await;
    assert_eq!(answer(&bench.ladder).await.0, FIRST);
    assert_eq!(first_probe.started(), 3, "a minute later the rate is free");
}

#[tokio::test(start_paused = true)]
async fn a_probe_held_back_by_the_rate_keeps_its_slot_for_the_next_request() {
    let (flaky, flaky_probe) = limited(free("flaky", vec![broken(10, Failure::Status)]), 2);
    let (spare, _) = free("spare", vec![text(10, SPARE)]);
    let breaker = BreakerSettings {
        open_s: 1,
        ..BREAKER
    };
    let bench = assembled(vec![flaky, spare], CapSettings::default(), breaker);

    for _ in 0..2 {
        assert_eq!(answer(&bench.ladder).await.0, SPARE);
    }
    tokio::time::advance(Duration::from_millis(1_500)).await;
    for _ in 0..2 {
        assert_eq!(answer(&bench.ladder).await.0, SPARE);
    }
    assert_eq!(flaky_probe.started(), 2);
    assert_eq!(
        results_of(&bench, "flaky"),
        [
            "failed_status",
            "failed_status",
            "skipped_rate",
            "skipped_rate"
        ]
    );

    tokio::time::advance(Duration::from_secs(60)).await;
    assert_eq!(answer(&bench.ladder).await.0, SPARE);
    assert_eq!(
        flaky_probe.started(),
        3,
        "the probe runs once the rate allows"
    );
}
