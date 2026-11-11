use futures::poll;

use super::*;

fn quick_to_probe(providers: Vec<Provider>) -> Bench {
    let breaker = BreakerSettings {
        open_s: 1,
        ..BREAKER
    };
    assembled(providers, CapSettings::default(), breaker)
}

async fn half_open_with_a_run_from_before(bench: &Bench) -> (AnswerStream, AnswerStream) {
    let breaker = &bench.ladder.steps[0].breaker;
    let mut early = bench.ladder.stream(request(), any_text()).await.unwrap();
    assert!(poll!(early.next()).is_pending());
    breaker.failed(Pass::Regular, Instant::now());
    breaker.failed(Pass::Regular, Instant::now());
    tokio::time::advance(Duration::from_millis(1_500)).await;
    let mut probing = bench.ladder.stream(request(), any_text()).await.unwrap();
    assert!(poll!(probing.next()).is_pending());
    (early, probing)
}

#[tokio::test(start_paused = true)]
async fn repeated_failures_open_the_breaker_and_the_step_is_skipped() {
    let (flaky, flaky_probe) = paid("flaky", vec![broken(10, Failure::Status)]);
    let (steady, _) = free("steady", vec![text(10, "{\"say\":\"ok\"}")]);
    let bench = uncapped(vec![flaky, steady]);

    for _ in 0..2 {
        assert!(answer(&bench.ladder).await.1.is_ok());
    }
    assert_eq!(flaky_probe.started(), 2);
    let (_, outcome) = answer(&bench.ladder).await;
    assert_eq!(outcome.unwrap().elapsed, Duration::from_millis(10));
    assert_eq!(flaky_probe.started(), 2, "an open breaker skips the step");
    assert_eq!(bench.seen.steps().last().unwrap().0, "steady");
    assert!(
        bench
            .seen
            .steps()
            .contains(&("flaky".to_owned(), "skipped_breaker"))
    );

    tokio::time::advance(Duration::from_secs(91)).await;
    assert!(answer(&bench.ladder).await.1.is_ok());
    assert_eq!(flaky_probe.started(), 3, "after the pause one call probes");
    assert!(answer(&bench.ladder).await.1.is_ok());
    assert_eq!(flaky_probe.started(), 3, "the failed probe reopened it");
}

#[tokio::test(start_paused = true)]
async fn the_provider_faults_open_the_breaker() {
    for failure in [Failure::Status, Failure::Transport, Failure::Timeout] {
        let (only, probe) = free("only", vec![broken(10, failure)]);
        let bench = uncapped(vec![only]);
        for _ in 0..2 {
            assert_eq!(answer(&bench.ladder).await.1, Err(LadderError::Exhausted));
        }
        assert_eq!(
            answer(&bench.ladder).await.1,
            Err(LadderError::Unavailable),
            "{failure:?}"
        );
        assert_eq!(probe.started(), 2);
    }
}

#[tokio::test(start_paused = true)]
async fn a_failure_that_depends_on_the_request_never_opens_the_breaker() {
    let scripts = [
        vec![broken(10, Failure::Request)],
        vec![broken(10, Failure::Refused)],
        vec![text(10, "{\"say\":\"cut"), broken(10, Failure::Length)],
        vec![text(10, "I would rather answer in prose.")],
        vec![text(10, "{\"say\":\"no closing brace")],
    ];
    for script in scripts {
        let (asked, probe) = free("asked", script.clone());
        let bench = uncapped(vec![asked]);
        for _ in 0..6 {
            assert!(answer(&bench.ladder).await.1.is_err());
        }
        assert_eq!(probe.started(), 6, "{script:?}");
        assert!(
            !bench
                .seen
                .steps()
                .iter()
                .any(|(_, result)| *result == "skipped_breaker")
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_probe_that_fails_because_of_its_request_leaves_the_breaker_half_open() {
    let (asked, probe) = free("asked", vec![broken(10, Failure::Request)]);
    let bench = uncapped(vec![asked]);
    let breaker = &bench.ladder.steps[0].breaker;
    breaker.failed(Pass::Regular, Instant::now());
    breaker.failed(Pass::Regular, Instant::now());
    assert_eq!(answer(&bench.ladder).await.1, Err(LadderError::Unavailable));

    tokio::time::advance(Duration::from_secs(91)).await;
    for _ in 0..3 {
        assert_eq!(answer(&bench.ladder).await.1, Err(LadderError::Exhausted));
    }

    assert_eq!(probe.started(), 3, "every probe went through");
}

#[tokio::test(start_paused = true)]
async fn a_run_from_before_the_breaker_opened_cannot_free_the_probe_slot() {
    let (slow, probe) = free("slow", vec![text(4_000, "{\"say\":\"late\"}")]);
    let bench = quick_to_probe(vec![slow]);
    let (early, _probing) = half_open_with_a_run_from_before(&bench).await;
    assert_eq!(probe.started(), 2);

    drop(early);

    assert_eq!(
        bench.ladder.stream(request(), any_text()).await.err(),
        Some(LadderError::Unavailable)
    );
    assert_eq!(probe.started(), 2, "the probe still holds the only slot");
}

#[tokio::test(start_paused = true)]
async fn an_answer_that_began_before_the_breaker_opened_does_not_close_it() {
    let (slow, probe) = free("slow", vec![text(4_000, "{\"say\":\"late\"}")]);
    let bench = quick_to_probe(vec![slow]);
    let (mut early, mut probing) = half_open_with_a_run_from_before(&bench).await;

    tokio::time::advance(Duration::from_millis(2_600)).await;
    assert!(matches!(early.next().await, Some(Ok(Event::Delta(_)))));
    assert!(matches!(early.next().await, Some(Ok(Event::Done(_)))));

    assert_eq!(
        bench.ladder.stream(request(), any_text()).await.err(),
        Some(LadderError::Unavailable),
        "only the probe closes it"
    );
    assert!(matches!(probing.next().await, Some(Ok(Event::Delta(_)))));
    assert!(matches!(probing.next().await, Some(Ok(Event::Done(_)))));
    assert!(answer(&bench.ladder).await.1.is_ok());
    assert_eq!(probe.started(), 3);
}

#[tokio::test(start_paused = true)]
async fn a_ladder_with_every_breaker_open_is_unavailable() {
    let (only, _) = free("only", vec![broken(10, Failure::Status)]);
    let bench = uncapped(vec![only]);
    for _ in 0..2 {
        assert_eq!(answer(&bench.ladder).await.1, Err(LadderError::Exhausted));
    }
    assert_eq!(answer(&bench.ladder).await.1, Err(LadderError::Unavailable));
}

#[tokio::test(start_paused = true)]
async fn a_ladder_without_steps_is_unavailable() {
    let bench = uncapped(Vec::new());
    assert_eq!(answer(&bench.ladder).await.1, Err(LadderError::Unavailable));
}
