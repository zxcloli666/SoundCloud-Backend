use super::*;

fn estimate(received: usize) -> u64 {
    Price::per_million_tokens(2_000_000, 8_000_000)
        .cost(&Usage::estimated(request().size(), received))
}

async fn settled(bench: &Bench) -> Vec<u64> {
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    bench.spend.counters().values().copied().collect()
}

#[tokio::test(start_paused = true)]
async fn a_priced_answer_is_charged_to_the_ladder_and_the_user() {
    let (first, _) = paid(
        "first",
        vec![
            usage(1_000, 1),
            text(10, "{\"say\":\"hi\"}"),
            usage(1_000, 100),
        ],
    );
    let caps = CapSettings {
        month_usd: Some(8.0),
        day_usd: Some(1.0),
        user_month_usd: Some(0.5),
        user_day_usd: Some(0.05),
    };
    let bench = bench(vec![first], caps);

    let (_, outcome) = answer(&bench.ladder).await;

    assert_eq!(outcome.unwrap().cost_micro_usd, 2_800);
    let counters = bench.spend.counters();
    assert_eq!(counters.len(), 4);
    assert!(
        counters.values().all(|spent| *spent == 2_800),
        "{counters:?}"
    );
    assert!(
        counters
            .keys()
            .any(|key| key.starts_with("chat.user.42.day."))
    );
}

#[tokio::test(start_paused = true)]
async fn a_reached_cap_skips_priced_steps_and_the_free_one_answers() {
    let (priced, priced_probe) = paid("priced", vec![text(10, "{\"say\":\"paid\"}")]);
    let (gratis, _) = free("gratis", vec![text(10, "{\"say\":\"free\"}")]);
    let caps = CapSettings {
        user_day_usd: Some(0.0001),
        ..CapSettings::default()
    };
    let bench = bench(vec![priced, gratis], caps);
    let today = chrono::Utc::now().format("%Y-%m-%d");
    bench.spend.set(&format!("chat.user.42.day.{today}"), 100);

    let (document, outcome) = answer(&bench.ladder).await;

    assert_eq!(document, "{\"say\":\"free\"}");
    assert_eq!(outcome.unwrap().provider, "gratis");
    assert_eq!(priced_probe.started(), 0);
    assert_eq!(
        bench.seen.steps(),
        steps(&[("priced", "skipped_budget"), ("gratis", "won")])
    );
    let other_user = Request {
        budget_user: Some("43".to_owned()),
        ..request()
    };
    let mut events = bench.ladder.stream(other_user, any_text()).await.unwrap();
    assert_eq!(
        events.next().await,
        Some(Ok(Event::Delta("{\"say\":\"paid\"}".to_owned())))
    );
}

#[tokio::test(start_paused = true)]
async fn a_spent_budget_with_only_priced_steps_is_unavailable() {
    let (priced, _) = paid("priced", vec![text(10, "{\"say\":\"paid\"}")]);
    let caps = CapSettings {
        month_usd: Some(0.0),
        ..CapSettings::default()
    };
    let bench = bench(vec![priced], caps);
    assert_eq!(answer(&bench.ladder).await.1, Err(LadderError::Unavailable));
}

#[tokio::test(start_paused = true)]
async fn an_unreadable_budget_skips_priced_steps_and_is_not_asked_again_for_a_while() {
    let (priced, priced_probe) = paid("priced", vec![text(10, "{\"say\":\"paid\"}")]);
    let (gratis, _) = free("gratis", vec![text(10, "{\"say\":\"free\"}")]);
    let bench = bench(vec![priced, gratis], monthly());
    bench.spend.go_down();

    for _ in 0..3 {
        assert_eq!(answer(&bench.ladder).await.1.unwrap().provider, "gratis");
    }
    assert_eq!(bench.spend.reads(), 1, "an outage is remembered");

    bench.spend.come_back();
    tokio::time::advance(Duration::from_secs(11)).await;
    assert_eq!(answer(&bench.ladder).await.1.unwrap().provider, "priced");
    assert_eq!(bench.spend.reads(), 2);
    assert_eq!(priced_probe.started(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_loser_is_charged_for_the_tokens_it_reported() {
    let (slow, _) = paid(
        "slow",
        vec![usage(1_000, 0), text(8_000, "{\"say\":\"late\"}")],
    );
    let (quick, _) = free("quick", vec![text(100, "{\"say\":\"quick\"}")]);
    let bench = bench(vec![slow, quick], monthly());

    assert!(answer(&bench.ladder).await.1.is_ok());

    assert_eq!(settled(&bench).await, [2_000]);
    assert_eq!(bench.seen.reports()[0].basis, "reported");
}

#[tokio::test(start_paused = true)]
async fn a_losing_priced_step_that_reported_no_usage_is_charged_an_estimate() {
    let (slow, _) = paid("slow", vec![text(8_000, "{\"say\":\"late\"}")]);
    let (quick, _) = free("quick", vec![text(100, "{\"say\":\"quick\"}")]);
    let bench = bench(vec![slow, quick], monthly());

    assert!(answer(&bench.ladder).await.1.is_ok());

    assert!(estimate(0) > 0);
    assert_eq!(settled(&bench).await, [estimate(0)]);
    let loser = &bench.seen.reports()[0];
    assert_eq!((loser.result, loser.basis), ("lost", "estimated"));
    assert_eq!(loser.usage, Usage::estimated(request().size(), 0));
}

#[tokio::test(start_paused = true)]
async fn a_winner_that_reported_no_usage_is_charged_an_estimate() {
    let said = "<think>so</think>{\"say\":\"hi\"}";
    let (first, _) = paid("first", vec![text(10, said)]);
    let bench = bench(vec![first], monthly());

    let outcome = answer(&bench.ladder).await.1.unwrap();

    assert_eq!(outcome.basis, Basis::Estimated);
    assert_eq!(outcome.cost_micro_usd, estimate(said.len()));
    assert_eq!(settled(&bench).await, [estimate(said.len())]);
}

#[tokio::test(start_paused = true)]
async fn a_client_that_leaves_mid_answer_is_charged_for_what_was_sent_and_heard() {
    let heard = "{\"say\":\"so";
    let (talker, _) = step(
        "talker",
        Price::per_million_tokens(2_000_000, 8_000_000),
        vec![text(10, heard)],
        true,
    );
    let bench = bench(vec![talker], monthly());

    let mut events = bench.ladder.stream(request(), any_text()).await.unwrap();
    assert!(matches!(events.next().await, Some(Ok(Event::Delta(_)))));
    drop(events);

    assert_eq!(settled(&bench).await, [estimate(heard.len())]);
    let report = &bench.seen.reports()[0];
    assert_eq!((report.result, report.basis), ("cancelled", "estimated"));
}

#[tokio::test(start_paused = true)]
async fn a_step_cut_by_the_deadline_is_charged_an_estimate() {
    let (silent, _) = step(
        "silent",
        Price::per_million_tokens(2_000_000, 8_000_000),
        Vec::new(),
        true,
    );
    let bench = bench(vec![silent], monthly());
    let short = Request {
        deadline: Some(HEDGE * 2),
        ..request()
    };

    let mut events = bench.ladder.stream(short, any_text()).await.unwrap();
    assert_eq!(events.next().await, Some(Err(LadderError::Deadline)));
    drop(events);

    assert_eq!(settled(&bench).await, [estimate(0)]);
    let report = &bench.seen.reports()[0];
    assert_eq!(
        (report.result, report.basis),
        ("failed_timeout", "estimated")
    );
}

#[tokio::test(start_paused = true)]
async fn a_step_refused_before_it_answered_costs_nothing() {
    for failure in [Failure::Status, Failure::Request, Failure::Transport] {
        let (refused, _) = paid("refused", vec![broken(10, failure)]);
        let bench = bench(vec![refused], monthly());

        assert_eq!(answer(&bench.ladder).await.1, Err(LadderError::Exhausted));

        assert_eq!(settled(&bench).await, Vec::<u64>::new(), "{failure:?}");
        let report = &bench.seen.reports()[0];
        assert_eq!((report.basis, report.cost_micro_usd), ("reported", 0));
    }
}

#[tokio::test(start_paused = true)]
async fn a_failure_after_the_provider_took_the_request_is_charged_an_estimate() {
    let (broke, _) = paid("broke", vec![accepted(), broken(10, Failure::Transport)]);
    let bench = bench(vec![broke], monthly());

    assert_eq!(answer(&bench.ladder).await.1, Err(LadderError::Exhausted));

    assert_eq!(settled(&bench).await, [estimate(0)]);
    let report = &bench.seen.reports()[0];
    assert_eq!(
        (report.result, report.basis),
        ("failed_transport", "estimated")
    );
}

#[tokio::test(start_paused = true)]
async fn a_free_step_without_usage_stays_free() {
    let (gratis, _) = free("gratis", vec![text(10, "{\"say\":\"free\"}")]);
    let bench = bench(vec![gratis], monthly());

    let outcome = answer(&bench.ladder).await.1.unwrap();

    assert_eq!(
        (outcome.basis, outcome.cost_micro_usd),
        (Basis::Reported, 0)
    );
    assert!(settled(&bench).await.is_empty());
}
