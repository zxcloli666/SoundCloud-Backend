use super::*;

#[tokio::test(start_paused = true)]
async fn a_group_starts_together_and_its_first_answer_wins() {
    let (first, _) = hanging("first", vec![]);
    let (fast, fast_probe) = free("fast", vec![text(6_000, "{\"say\":\"fast\"}")]);
    let (think, think_probe) = free("think", vec![text(2_000, "{\"say\":\"think\"}")]);
    let (local, local_probe) = free("local", vec![text(4_000, "{\"say\":\"local\"}")]);
    let bench = grouped(
        vec![first, fast, think, local],
        vec![vec![0], vec![1, 2, 3]],
    );

    let (document, outcome) = answer(&bench.ladder).await;
    let outcome = outcome.unwrap();

    assert_eq!(document, "{\"say\":\"think\"}");
    assert_eq!(outcome.provider, "think");
    assert_eq!(outcome.elapsed, HEDGE + Duration::from_millis(2_000));
    for probe in [&fast_probe, &think_probe, &local_probe] {
        assert_eq!(probe.time_left(), [DEADLINE - HEDGE]);
    }
    assert_eq!((fast_probe.dropped(), local_probe.dropped()), (1, 1));
    assert_eq!(
        bench.seen.steps(),
        steps(&[
            ("local", "lost"),
            ("fast", "lost"),
            ("first", "lost"),
            ("think", "won"),
        ])
    );
}

#[tokio::test(start_paused = true)]
async fn a_failed_member_leaves_the_ladder_where_it_is_while_its_group_runs() {
    let (broken_one, _) = free("broken", vec![broken(500, Failure::Status)]);
    let (slow, _) = free(
        "slow",
        vec![text(100, "{\"say\":"), text(3_000, "\"done\"}")],
    );
    let (next, next_probe) = free("next", vec![text(1, "{\"say\":\"next\"}")]);
    let bench = grouped(vec![broken_one, slow, next], vec![vec![0, 1], vec![2]]);
    let done: Commit = Arc::new(|_, held: &str| held.contains("done"));

    let (document, outcome) = answer_once(&bench.ladder, done).await;

    assert_eq!(document, "{\"say\":\"done\"}");
    assert_eq!(outcome.unwrap().elapsed, Duration::from_millis(3_100));
    assert_eq!(next_probe.started(), 0);
    assert_eq!(
        bench.seen.steps(),
        steps(&[("broken", "failed_status"), ("slow", "won")])
    );
}

#[tokio::test(start_paused = true)]
async fn a_group_that_failed_whole_hands_over_at_once() {
    let (status, _) = free("status", vec![broken(1_300, Failure::Status)]);
    let (transport, _) = free("transport", vec![broken(1_800, Failure::Transport)]);
    let (next, next_probe) = free("next", vec![text(100, "{\"say\":\"next\"}")]);
    let bench = grouped(vec![status, transport, next], vec![vec![0, 1], vec![2]]);

    let (document, outcome) = answer(&bench.ladder).await;

    assert_eq!(document, "{\"say\":\"next\"}");
    assert_eq!(outcome.unwrap().elapsed, Duration::from_millis(1_900));
    assert_eq!(
        next_probe.time_left(),
        [DEADLINE - Duration::from_millis(1_800)]
    );
    assert_eq!(
        bench.seen.steps(),
        steps(&[
            ("status", "failed_status"),
            ("transport", "failed_transport"),
            ("next", "won"),
        ])
    );
}

#[tokio::test(start_paused = true)]
async fn a_silent_group_is_hedged_like_one_step() {
    let (one, _) = hanging("one", vec![]);
    let (two, _) = hanging("two", vec![]);
    let (next, next_probe) = free("next", vec![text(100, "{\"say\":\"next\"}")]);
    let bench = grouped(vec![one, two, next], vec![vec![0, 1], vec![2]]);

    let (document, outcome) = answer(&bench.ladder).await;

    assert_eq!(document, "{\"say\":\"next\"}");
    assert_eq!(outcome.unwrap().elapsed, HEDGE + Duration::from_millis(100));
    assert_eq!(next_probe.time_left(), [DEADLINE - HEDGE]);
    assert_eq!(
        bench.seen.steps(),
        steps(&[("two", "lost"), ("one", "lost"), ("next", "won")])
    );
}
