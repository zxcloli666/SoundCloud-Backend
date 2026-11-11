use super::*;

#[tokio::test(start_paused = true)]
async fn complete_returns_the_typed_answer_of_the_first_valid_step() {
    let (first, _) = paid(
        "first",
        vec![usage(10, 5), text(10, "```json\n{\"say\":\"typed\"}\n```")],
    );
    let bench = uncapped(vec![first]);

    let completion: Completion<Said> = bench.ladder.complete(request()).await.unwrap();

    assert_eq!(completion.value.say, "typed");
    assert_eq!(completion.outcome.provider, "first");
    assert_eq!(completion.outcome.cost_micro_usd, 60);
}

#[tokio::test(start_paused = true)]
async fn complete_moves_on_when_an_answer_does_not_fit_the_type() {
    let (wrong, _) = free("wrong", vec![text(10, "{\"speech\":\"nope\"}")]);
    let (cut, _) = free(
        "cut",
        vec![text(10, "{\"say\":\"ha"), broken(10, Failure::Transport)],
    );
    let (right, _) = free("right", vec![text(10, "{\"say\":\"yes\"}")]);
    let bench = uncapped(vec![wrong, cut, right]);

    let completion: Completion<Said> = bench.ladder.complete(request()).await.unwrap();

    assert_eq!(completion.value.say, "yes");
    assert_eq!(
        bench.seen.steps(),
        steps(&[
            ("wrong", "failed_invalid"),
            ("cut", "failed_transport"),
            ("right", "won"),
        ])
    );
}

#[tokio::test(start_paused = true)]
async fn complete_comes_back_to_a_hedge_it_cancelled_when_the_winner_fails() {
    let (first, _) = free(
        "first",
        vec![text(2_500, "{\"say\":"), broken(10, Failure::Transport)],
    );
    let (second, second_probe) = free("second", vec![text(1_000, "{\"say\":\"second\"}")]);
    let bench = uncapped(vec![first, second]);

    let completion: Completion<Said> = bench.ladder.complete(request()).await.unwrap();

    assert_eq!(completion.value.say, "second");
    assert_eq!(second_probe.started(), 2);
    assert_eq!(
        bench.seen.steps(),
        steps(&[
            ("second", "lost"),
            ("first", "failed_transport"),
            ("second", "won"),
        ])
    );
}

#[tokio::test(start_paused = true)]
async fn complete_reports_exhaustion_when_no_step_gives_a_valid_answer() {
    let (wrong, _) = free("wrong", vec![text(10, "[1, 2, 3]")]);
    let bench = uncapped(vec![wrong]);
    let failed = bench.ladder.complete::<Said>(request()).await.unwrap_err();
    assert_eq!(failed, LadderError::Exhausted);
}
