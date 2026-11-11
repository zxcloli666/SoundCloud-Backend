use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::future::pending;
use serde::Deserialize;
use tokio::time::sleep;

use super::*;
use crate::budget::memory::MemorySpend;
use crate::config::{BreakerSettings, CapSettings};
use crate::observer::Failure;
use crate::provider::{Backend, Price, StepEvent, StepStream};
use crate::request::{Message, System};
use crate::schema::Schema;

const HEDGE: Duration = Duration::from_secs(2);
const DEADLINE: Duration = Duration::from_secs(10);
const STEP_TIMEOUT: Duration = Duration::from_secs(9);

type Script = Vec<(u64, Result<StepEvent, Failure>)>;

#[derive(Clone, Default)]
struct Probe {
    started: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
}

struct Scripted {
    script: Script,
    hangs: bool,
    probe: Probe,
}

struct Leaving(Arc<AtomicUsize>);

#[derive(Default)]
struct Seen(Mutex<Vec<(String, &'static str)>>);

struct Bench {
    ladder: Arc<Ladder>,
    seen: Arc<Seen>,
    spend: Arc<MemorySpend>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Said {
    say: String,
}

impl Backend for Scripted {
    fn start(&self, _request: Arc<Request>) -> StepStream {
        self.probe.started.fetch_add(1, Ordering::SeqCst);
        let state = (
            self.script.clone().into_iter(),
            Leaving(self.probe.dropped.clone()),
            self.hangs,
        );
        stream::unfold(state, |(mut script, leaving, hangs)| async move {
            match script.next() {
                Some((delay, event)) => {
                    sleep(Duration::from_millis(delay)).await;
                    Some((event, (script, leaving, hangs)))
                }
                None if hangs => pending().await,
                None => None,
            }
        })
        .boxed()
    }
}

impl Drop for Leaving {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl Observer for Seen {
    fn step(&self, report: &StepReport<'_>) {
        self.0
            .lock()
            .unwrap()
            .push((report.provider.to_owned(), report.result.as_str()));
    }
}

impl Seen {
    fn steps(&self) -> Vec<(String, &'static str)> {
        self.0.lock().unwrap().clone()
    }
}

impl Probe {
    fn started(&self) -> usize {
        self.started.load(Ordering::SeqCst)
    }

    fn dropped(&self) -> usize {
        self.dropped.load(Ordering::SeqCst)
    }
}

fn text(delay: u64, piece: &str) -> (u64, Result<StepEvent, Failure>) {
    (delay, Ok(StepEvent::Text(piece.to_owned())))
}

fn usage(input_tokens: u64, output_tokens: u64) -> (u64, Result<StepEvent, Failure>) {
    (
        0,
        Ok(StepEvent::Usage(Usage {
            input_tokens,
            output_tokens,
            ..Usage::default()
        })),
    )
}

fn broken(delay: u64, failure: Failure) -> (u64, Result<StepEvent, Failure>) {
    (delay, Err(failure))
}

fn free(id: &str, script: Script) -> (Provider, Probe) {
    step(id, Price::default(), script, false)
}

fn paid(id: &str, script: Script) -> (Provider, Probe) {
    step(
        id,
        Price::per_million_tokens(2_000_000, 8_000_000),
        script,
        false,
    )
}

fn hanging(id: &str, script: Script) -> (Provider, Probe) {
    step(id, Price::default(), script, true)
}

fn step(id: &str, price: Price, script: Script, hangs: bool) -> (Provider, Probe) {
    let probe = Probe::default();
    let backend = Scripted {
        script,
        hangs,
        probe: probe.clone(),
    };
    (
        Provider::with_backend(id, price, STEP_TIMEOUT, Box::new(backend)),
        probe,
    )
}

fn bench(providers: Vec<Provider>, caps: CapSettings) -> Bench {
    let seen = Arc::new(Seen::default());
    let spend = Arc::new(MemorySpend::default());
    let caps = Caps::from(caps);
    let breaker = BreakerSettings {
        failures: 2,
        window_s: 30,
        open_s: 90,
    };
    let ladder = Ladder {
        id: "chat".to_owned(),
        steps: providers
            .into_iter()
            .map(|provider| Step {
                provider: Arc::new(provider),
                breaker: Breaker::new(breaker),
            })
            .collect(),
        hedge_after: HEDGE,
        deadline: DEADLINE,
        budget: caps
            .is_set()
            .then(|| Arc::new(Budget::new(spend.clone(), "chat", caps))),
        observer: seen.clone(),
    };
    Bench {
        ladder: Arc::new(ladder),
        seen,
        spend,
    }
}

fn uncapped(providers: Vec<Provider>) -> Bench {
    bench(providers, CapSettings::default())
}

fn request() -> Request {
    Request {
        system: System::fixed("rules"),
        messages: vec![Message::user("hello")],
        schema: Schema::object([("say", Schema::String)]),
        budget_user: Some("42".to_owned()),
        deadline: None,
    }
}

async fn answer(ladder: &Arc<Ladder>) -> (String, Result<Outcome, LadderError>) {
    let mut events = match ladder.stream(request()).await {
        Ok(events) => events,
        Err(error) => return (String::new(), Err(error)),
    };
    let mut document = String::new();
    while let Some(event) = events.next().await {
        match event {
            Ok(Event::Delta(delta)) => document.push_str(&delta),
            Ok(Event::Done(outcome)) => return (document, Ok(outcome)),
            Err(error) => return (document, Err(error)),
        }
    }
    panic!("the stream ended without a final item");
}

fn steps(pairs: &[(&str, &'static str)]) -> Vec<(String, &'static str)> {
    pairs
        .iter()
        .map(|(provider, result)| ((*provider).to_owned(), *result))
        .collect()
}

#[tokio::test(start_paused = true)]
async fn the_first_step_answers_and_the_next_never_starts() {
    let (first, _) = paid(
        "first",
        vec![
            usage(1_000, 1),
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
    assert_eq!(outcome.first_output, Duration::from_millis(900));
    assert_eq!(outcome.elapsed, Duration::from_millis(950));
    assert_eq!(second_probe.started(), 0);
    assert_eq!(bench.seen.steps(), steps(&[("first", "won")]));
}

#[tokio::test(start_paused = true)]
async fn a_silent_step_is_hedged_and_the_first_output_wins() {
    let (slow, slow_probe) = paid(
        "slow",
        vec![usage(400, 1), text(8_000, "{\"say\":\"late\"}")],
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
            usage(1_000, 1),
            text(100, "{\"say\":\"hi\"}"),
            broken(50, Failure::Transport),
        ],
    );
    let bench = uncapped(vec![first]);

    let (document, outcome) = answer(&bench.ladder).await;

    assert_eq!(document, "{\"say\":\"hi\"}");
    assert_eq!(outcome.unwrap().cost_micro_usd, 2_008);
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
async fn the_deadline_ends_a_ladder_that_stays_silent() {
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
        steps(&[("first", "failed_timeout"), ("second", "cancelled")])
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
        bench.ladder.stream(short).await.unwrap().next().await,
        Some(Err(LadderError::Deadline))
    ));
    assert_eq!(started.elapsed(), Duration::from_secs(1));

    let (first, _) = hanging("first", vec![]);
    let bench = uncapped(vec![first]);
    let started = Instant::now();
    let long = Request {
        deadline: Some(Duration::from_secs(600)),
        ..request()
    };
    let mut events = bench.ladder.stream(long).await.unwrap();
    assert_eq!(events.next().await, Some(Err(LadderError::Exhausted)));
    assert_eq!(started.elapsed(), STEP_TIMEOUT);
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
    let mut events = bench.ladder.stream(other_user).await.unwrap();
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
async fn an_unreadable_budget_skips_priced_steps() {
    let (priced, _) = paid("priced", vec![text(10, "{\"say\":\"paid\"}")]);
    let (gratis, _) = free("gratis", vec![text(10, "{\"say\":\"free\"}")]);
    let caps = CapSettings {
        month_usd: Some(8.0),
        ..CapSettings::default()
    };
    let bench = bench(vec![priced, gratis], caps);
    bench.spend.go_down();
    assert_eq!(answer(&bench.ladder).await.1.unwrap().provider, "gratis");
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_loser_is_charged_for_the_tokens_it_reported() {
    let (slow, _) = paid(
        "slow",
        vec![usage(1_000, 0), text(8_000, "{\"say\":\"late\"}")],
    );
    let (quick, _) = free("quick", vec![text(100, "{\"say\":\"quick\"}")]);
    let caps = CapSettings {
        month_usd: Some(8.0),
        ..CapSettings::default()
    };
    let bench = bench(vec![slow, quick], caps);

    assert!(answer(&bench.ladder).await.1.is_ok());
    tokio::task::yield_now().await;

    assert_eq!(
        bench.spend.counters().values().copied().collect::<Vec<_>>(),
        [2_000]
    );
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

    let mut events = bench.ladder.stream(request()).await.unwrap();
    assert_eq!(
        events.next().await,
        Some(Ok(Event::Delta("{\"say\":\"so".to_owned())))
    );
    assert_eq!(probe.dropped(), 0);
    drop(events);

    assert_eq!(probe.dropped(), 1);
    assert_eq!(bench.seen.steps(), steps(&[("talker", "cancelled")]));
}

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
