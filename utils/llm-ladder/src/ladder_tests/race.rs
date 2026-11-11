use super::*;

#[tokio::test(start_paused = true)]
async fn the_first_step_answers_and_the_next_never_starts() {
    let (first, _) = paid(
        "first",
        vec![
            usage_so_far(1_000, 1),
            text(900, "{\"say\":"),
            text(50, "\"hi\"}"),
            usage(1_000, 100),
        ],
    );
    let (second, second_probe) = free("second", vec![text(100, "{\"say\":\"no\"}")]);
    let bench = uncapped(vec![first, second]);

    let (document, outcome) = answer(&bench.ladder).await;
    let outcome = outcome.unwrap();

    assert_eq!(document, "{\"say\":\"hi\"}");
    assert_eq!(outcome.provider, "first");
    assert_eq!(
        outcome.usage,
        Usage {
            input_tokens: 1_000,
            output_tokens: 100,
            ..Usage::default()
        }
    );
    assert_eq!(outcome.cost_micro_usd, 2_800);
    assert_eq!(outcome.basis, Basis::Reported);
    assert_eq!(outcome.first_output, Duration::from_millis(900));
    assert_eq!(outcome.elapsed, Duration::from_millis(950));
    assert_eq!(second_probe.started(), 0);
    assert_eq!(bench.seen.steps(), steps(&[("first", "won")]));
}

#[tokio::test(start_paused = true)]
async fn a_failed_step_starts_the_next_one_at_once() {
    let (down, _) = paid("down", vec![broken(200, Failure::Status)]);
    let (up, _) = free("up", vec![text(300, "{\"say\":\"up\"}")]);
    let bench = uncapped(vec![down, up]);

    let (document, outcome) = answer(&bench.ladder).await;
    let outcome = outcome.unwrap();

    assert_eq!(document, "{\"say\":\"up\"}");
    assert_eq!(outcome.elapsed, Duration::from_millis(500));
    assert_eq!(
        bench.seen.steps(),
        steps(&[("down", "failed_status"), ("up", "won")])
    );
}

#[tokio::test(start_paused = true)]
async fn every_step_failing_before_output_exhausts_the_ladder() {
    let (first, _) = free("first", vec![broken(10, Failure::Transport)]);
    let (second, _) = free("second", vec![]);
    let (third, _) = free("third", vec![text(5, "no json here")]);
    let bench = uncapped(vec![first, second, third]);

    let (document, outcome) = answer(&bench.ladder).await;

    assert_eq!(document, "");
    assert_eq!(outcome, Err(LadderError::Exhausted));
    assert_eq!(
        bench.seen.steps(),
        steps(&[
            ("first", "failed_transport"),
            ("second", "failed_invalid"),
            ("third", "failed_invalid"),
        ])
    );
}

#[tokio::test(start_paused = true)]
async fn a_failure_after_the_first_delta_is_an_error_not_a_switch() {
    let (first, _) = free(
        "first",
        vec![text(100, "{\"say\":\"ha"), broken(100, Failure::Transport)],
    );
    let (second, second_probe) = free("second", vec![text(1, "{\"say\":\"other\"}")]);
    let bench = uncapped(vec![first, second]);

    let (document, outcome) = answer(&bench.ladder).await;

    assert_eq!(document, "{\"say\":\"ha");
    assert_eq!(outcome, Err(LadderError::ProviderFailed));
    assert_eq!(second_probe.started(), 0);
    assert_eq!(bench.seen.steps(), steps(&[("first", "failed_transport")]));
}

#[tokio::test(start_paused = true)]
async fn an_answer_cut_before_its_closing_brace_is_a_failure() {
    let (first, _) = free("first", vec![text(100, "{\"say\":\"cut off")]);
    let bench = uncapped(vec![first]);

    let (_, outcome) = answer(&bench.ladder).await;

    assert_eq!(outcome, Err(LadderError::ProviderFailed));
    assert_eq!(bench.seen.steps(), steps(&[("first", "failed_invalid")]));
}

#[tokio::test(start_paused = true)]
async fn a_whole_answer_stands_when_its_stream_breaks_afterwards() {
    let (first, _) = paid(
        "first",
        vec![
            usage_so_far(1_000, 1),
            text(100, "{\"say\":\"hi\"}"),
            broken(50, Failure::Transport),
        ],
    );
    let bench = uncapped(vec![first]);

    let (document, outcome) = answer(&bench.ladder).await;
    let outcome = outcome.unwrap();

    assert_eq!(document, "{\"say\":\"hi\"}");
    assert_eq!(
        (outcome.usage.input_tokens, outcome.usage.output_tokens),
        (1_000, 4),
        "the last usage never came: twelve bytes of answer count as four tokens"
    );
    assert_eq!(
        (outcome.basis, outcome.cost_micro_usd),
        (Basis::Estimated, 2_032)
    );
    assert_eq!(bench.seen.steps(), steps(&[("first", "won")]));
}

#[tokio::test(start_paused = true)]
async fn a_whole_answer_is_not_held_by_a_provider_that_keeps_the_line() {
    let (wordy, probe) = hanging(
        "wordy",
        vec![
            text(100, "{\"say\":\"hi\"}"),
            text(400, " And one more thing"),
        ],
    );
    let bench = uncapped(vec![wordy]);

    let (document, outcome) = answer(&bench.ladder).await;

    assert_eq!(document, "{\"say\":\"hi\"}");
    assert_eq!(outcome.unwrap().elapsed, Duration::from_millis(1_100));
    assert_eq!(probe.dropped(), 1);
    assert_eq!(bench.seen.steps(), steps(&[("wordy", "won")]));
}

#[tokio::test(start_paused = true)]
async fn the_deadline_ends_a_silent_ladder_and_blames_the_steps_that_stayed_silent() {
    let (first, first_probe) = hanging("first", vec![]);
    let (second, second_probe) = hanging("second", vec![]);
    let bench = uncapped(vec![first, second]);
    let started = Instant::now();

    let (_, outcome) = answer(&bench.ladder).await;

    assert_eq!(outcome, Err(LadderError::Deadline));
    assert_eq!(started.elapsed(), DEADLINE);
    assert_eq!((first_probe.dropped(), second_probe.dropped()), (1, 1));
    assert_eq!(
        bench.seen.steps(),
        steps(&[("first", "failed_timeout"), ("second", "failed_timeout")])
    );
}

#[tokio::test(start_paused = true)]
async fn a_step_started_just_before_the_deadline_is_not_blamed_for_it() {
    let (early, _) = free("early", vec![broken(8_500, Failure::Transport)]);
    let (late, late_probe) = unhedged(hanging("late", vec![]));
    let bench = uncapped(vec![early, late]);

    let (_, outcome) = answer(&bench.ladder).await;

    assert_eq!(outcome, Err(LadderError::Deadline));
    assert_eq!(late_probe.time_left(), [Duration::from_millis(1_500)]);
    assert_eq!(
        bench.seen.steps(),
        steps(&[("early", "failed_transport"), ("late", "cancelled")])
    );
}

#[tokio::test(start_paused = true)]
async fn a_silent_step_that_runs_into_the_deadline_again_and_again_opens_its_breaker() {
    let (silent, silent_probe) = hanging("silent", vec![]);
    let bench = uncapped(vec![silent]);
    let short = || Request {
        deadline: Some(HEDGE + Duration::from_secs(1)),
        ..request()
    };

    for _ in 0..2 {
        let mut events = bench.ladder.stream(short(), any_text()).await.unwrap();
        assert_eq!(events.next().await, Some(Err(LadderError::Deadline)));
    }

    assert_eq!(
        bench.seen.steps(),
        steps(&[("silent", "failed_timeout"), ("silent", "failed_timeout")])
    );
    assert_eq!(
        bench.ladder.stream(short(), any_text()).await.err(),
        Some(LadderError::Unavailable)
    );
    assert_eq!(silent_probe.started(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_step_that_cannot_stream_is_blamed_by_its_own_timeout_and_never_by_the_deadline() {
    let (whole, probe) = one_shot(hanging("whole", vec![]));
    let bench = uncapped(vec![whole]);
    let short = || Request {
        deadline: Some(HEDGE + Duration::from_secs(1)),
        ..request()
    };

    for _ in 0..3 {
        let mut events = bench.ladder.stream(short(), any_text()).await.unwrap();
        assert_eq!(events.next().await, Some(Err(LadderError::Deadline)));
    }
    assert_eq!(probe.started(), 3, "the deadline never opened its breaker");
    assert_eq!(bench.seen.steps(), steps(&[("whole", "cancelled"); 3]));

    assert_eq!(answer(&bench.ladder).await.1, Err(LadderError::Exhausted));
    assert_eq!(
        bench.seen.steps().last(),
        Some(&("whole".to_owned(), "failed_timeout"))
    );
}

#[tokio::test(start_paused = true)]
async fn a_request_can_shorten_the_deadline_but_not_extend_it() {
    let (first, _) = hanging("first", vec![]);
    let bench = uncapped(vec![first]);
    let started = Instant::now();
    let short = Request {
        deadline: Some(Duration::from_secs(1)),
        ..request()
    };
    assert!(matches!(
        bench
            .ladder
            .stream(short, any_text())
            .await
            .unwrap()
            .next()
            .await,
        Some(Err(LadderError::Deadline))
    ));
    assert_eq!(started.elapsed(), Duration::from_secs(1));
    assert_eq!(bench.seen.steps(), steps(&[("first", "cancelled")]));

    let (first, _) = hanging("first", vec![]);
    let bench = uncapped(vec![first]);
    let started = Instant::now();
    let long = Request {
        deadline: Some(Duration::from_secs(600)),
        ..request()
    };
    let mut events = bench.ladder.stream(long, any_text()).await.unwrap();
    assert_eq!(events.next().await, Some(Err(LadderError::Exhausted)));
    assert_eq!(started.elapsed(), STEP_TIMEOUT);
}

#[tokio::test(start_paused = true)]
async fn reasoning_fences_and_trailing_text_never_reach_the_caller() {
    let (wordy, _) = free(
        "wordy",
        vec![
            text(10, "<think>the user said hello, so {\"say\""),
            text(10, ": \"maybe\"}</think>\n```json\n"),
            text(10, "{\"say\":\"hi\"}"),
            text(10, "\n```\nHope that helps!"),
        ],
    );
    let bench = uncapped(vec![wordy]);

    let (document, outcome) = answer(&bench.ladder).await;

    assert_eq!(document, "{\"say\":\"hi\"}");
    assert_eq!(outcome.unwrap().first_output, Duration::from_millis(30));
}

#[tokio::test(start_paused = true)]
async fn dropping_the_answer_cancels_the_provider_call() {
    let (talker, probe) = hanging("talker", vec![text(10, "{\"say\":\"so")]);
    let bench = uncapped(vec![talker]);

    let mut events = bench.ladder.stream(request(), any_text()).await.unwrap();
    assert_eq!(
        events.next().await,
        Some(Ok(Event::Delta("{\"say\":\"so".to_owned())))
    );
    assert_eq!(probe.dropped(), 0);
    drop(events);

    assert_eq!(probe.dropped(), 1);
    assert_eq!(bench.seen.steps(), steps(&[("talker", "cancelled")]));
}
