use super::*;

#[tokio::test(start_paused = true)]
async fn a_silent_step_is_hedged_and_the_first_output_wins() {
    let (slow, slow_probe) = paid(
        "slow",
        vec![usage_so_far(400, 1), text(8_000, "{\"say\":\"late\"}")],
    );
    let (quick, _) = free("quick", vec![text(500, "{\"say\":\"quick\"}")]);
    let (spare, spare_probe) = free("spare", vec![text(1, "{\"say\":\"spare\"}")]);
    let bench = uncapped(vec![slow, quick, spare]);

    let (document, outcome) = answer(&bench.ladder).await;
    let outcome = outcome.unwrap();

    assert_eq!(document, "{\"say\":\"quick\"}");
    assert_eq!(outcome.provider, "quick");
    assert_eq!(outcome.first_output, Duration::from_millis(500));
    assert_eq!(outcome.elapsed, HEDGE + Duration::from_millis(500));
    assert_eq!(slow_probe.dropped(), 1, "the loser is cancelled");
    assert_eq!(spare_probe.started(), 0);
    assert_eq!(
        bench.seen.steps(),
        steps(&[("slow", "lost"), ("quick", "won")])
    );
}

#[tokio::test(start_paused = true)]
async fn a_slow_first_step_still_wins_when_it_speaks_before_the_hedge() {
    let (slow, _) = free("slow", vec![text(2_400, "{\"say\":\"mine\"}")]);
    let (hedge, hedge_probe) = free("hedge", vec![text(900, "{\"say\":\"theirs\"}")]);
    let bench = uncapped(vec![slow, hedge]);

    let (document, outcome) = answer(&bench.ladder).await;

    assert_eq!(document, "{\"say\":\"mine\"}");
    assert_eq!(outcome.unwrap().provider, "slow");
    assert_eq!((hedge_probe.started(), hedge_probe.dropped()), (1, 1));
    assert_eq!(
        bench.seen.steps(),
        steps(&[("hedge", "lost"), ("slow", "won")])
    );
}

#[tokio::test(start_paused = true)]
async fn a_step_that_outlives_its_own_timeout_gives_way() {
    let (stuck, _) = hanging("stuck", vec![]);
    let (ready, _) = free("ready", vec![text(7_500, "{\"say\":\"ok\"}")]);
    let bench = uncapped(vec![stuck, ready]);

    let (document, outcome) = answer(&bench.ladder).await;

    assert_eq!(document, "{\"say\":\"ok\"}");
    assert_eq!(
        outcome.unwrap().elapsed,
        HEDGE + Duration::from_millis(7_500)
    );
    assert_eq!(
        bench.seen.steps(),
        steps(&[("stuck", "failed_timeout"), ("ready", "won")])
    );
}

#[tokio::test(start_paused = true)]
async fn a_step_out_of_the_hedge_waits_for_the_silent_one_to_fail() {
    let (silent, _) = free("silent", vec![broken(7_000, Failure::Timeout)]);
    let (local, local_probe) = unhedged(free("local", vec![text(500, "{\"say\":\"local\"}")]));
    let bench = uncapped(vec![silent, local]);

    let (document, outcome) = answer(&bench.ladder).await;

    assert_eq!(document, "{\"say\":\"local\"}");
    assert_eq!(outcome.unwrap().elapsed, Duration::from_millis(7_500));
    assert_eq!(local_probe.time_left(), [DEADLINE - Duration::from_secs(7)]);
    assert_eq!(
        bench.seen.steps(),
        steps(&[("silent", "failed_timeout"), ("local", "won")])
    );
}

#[tokio::test(start_paused = true)]
async fn a_step_out_of_the_hedge_starts_only_when_every_running_step_has_failed() {
    let (first, _) = free("first", vec![broken(5_000, Failure::Status)]);
    let (second, second_probe) = free("second", vec![broken(1_000, Failure::Transport)]);
    let (local, local_probe) = unhedged(free("local", vec![text(100, "{\"say\":\"local\"}")]));
    let (after, after_probe) = free("after", vec![text(1, "{\"say\":\"after\"}")]);
    let bench = uncapped(vec![first, second, local, after]);

    let (document, outcome) = answer(&bench.ladder).await;

    assert_eq!(document, "{\"say\":\"local\"}");
    assert_eq!(outcome.unwrap().elapsed, Duration::from_millis(5_100));
    assert_eq!(second_probe.time_left(), [DEADLINE - HEDGE]);
    assert_eq!(local_probe.time_left(), [DEADLINE - Duration::from_secs(5)]);
    assert_eq!(after_probe.started(), 0, "the steps after it wait for it");
    assert_eq!(
        bench.seen.steps(),
        steps(&[
            ("second", "failed_transport"),
            ("first", "failed_status"),
            ("local", "won"),
        ])
    );
}

#[tokio::test(start_paused = true)]
async fn a_step_hears_the_time_left_of_its_own_timeout_or_of_the_ladder() {
    let (first, first_probe) = free("first", vec![broken(3_000, Failure::Status)]);
    let (second, second_probe) = free("second", vec![text(10, "{\"say\":\"ok\"}")]);
    let bench = uncapped(vec![first, second]);

    assert!(answer(&bench.ladder).await.1.is_ok());

    assert_eq!(first_probe.time_left(), [STEP_TIMEOUT]);
    assert_eq!(second_probe.time_left(), [DEADLINE - HEDGE]);
}
