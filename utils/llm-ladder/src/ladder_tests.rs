use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::future::pending;
use serde::Deserialize;
use tokio::time::sleep;

use super::*;
use crate::breaker::Pass;
use crate::budget::memory::MemorySpend;
use crate::config::{BreakerSettings, CapSettings};
use crate::observer::Failure;
use crate::provider::{Backend, Price, StepEvent, StepStream};
use crate::request::{Message, System};
use crate::schema::Schema;

#[path = "ladder_tests/breaker.rs"]
mod breaker;
#[path = "ladder_tests/commit.rs"]
mod commit;
#[path = "ladder_tests/complete.rs"]
mod complete;
#[path = "ladder_tests/hedge.rs"]
mod hedge;
#[path = "ladder_tests/race.rs"]
mod race;
#[path = "ladder_tests/spend.rs"]
mod spend;

const HEDGE: Duration = Duration::from_secs(2);
const DEADLINE: Duration = Duration::from_secs(10);
const STEP_TIMEOUT: Duration = Duration::from_secs(9);
const BREAKER: BreakerSettings = BreakerSettings {
    failures: 2,
    window_s: 30,
    open_s: 90,
};

type Script = Vec<(u64, Result<StepEvent, Failure>)>;

#[derive(Clone, Default)]
struct Probe {
    started: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
    time_left: Arc<Mutex<Vec<Duration>>>,
}

struct Scripted {
    script: Script,
    hangs: bool,
    probe: Probe,
}

struct Leaving(Arc<AtomicUsize>);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Seen {
    provider: String,
    result: &'static str,
    basis: &'static str,
    usage: Usage,
    cost_micro_usd: u64,
}

#[derive(Default)]
struct Watched(Mutex<Vec<Seen>>);

struct Bench {
    ladder: Arc<Ladder>,
    seen: Arc<Watched>,
    spend: Arc<MemorySpend>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Said {
    say: String,
}

impl Backend for Scripted {
    fn start(&self, _request: Arc<Request>, time_left: Duration) -> StepStream {
        self.probe.started.fetch_add(1, Ordering::SeqCst);
        self.probe.time_left.lock().unwrap().push(time_left);
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

impl Observer for Watched {
    fn step(&self, report: &StepReport<'_>) {
        self.0.lock().unwrap().push(Seen {
            provider: report.provider.to_owned(),
            result: report.result.as_str(),
            basis: report.basis.as_str(),
            usage: report.usage,
            cost_micro_usd: report.cost_micro_usd,
        });
    }
}

impl Watched {
    fn steps(&self) -> Vec<(String, &'static str)> {
        self.reports()
            .into_iter()
            .map(|seen| (seen.provider, seen.result))
            .collect()
    }

    fn reports(&self) -> Vec<Seen> {
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

    fn time_left(&self) -> Vec<Duration> {
        self.time_left.lock().unwrap().clone()
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

fn unhedged((mut provider, probe): (Provider, Probe)) -> (Provider, Probe) {
    provider.hedged = false;
    (provider, probe)
}

fn one_shot((mut provider, probe): (Provider, Probe)) -> (Provider, Probe) {
    provider.streams = false;
    (provider, probe)
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
    assembled(providers, caps, BREAKER)
}

fn assembled(providers: Vec<Provider>, caps: CapSettings, breaker: BreakerSettings) -> Bench {
    let seen = Arc::new(Watched::default());
    let spend = Arc::new(MemorySpend::default());
    let caps = Caps::from(caps);
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

fn monthly() -> CapSettings {
    CapSettings {
        month_usd: Some(8.0),
        ..CapSettings::default()
    }
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

fn any_text() -> Commit {
    Arc::new(|_| true)
}

async fn answer(ladder: &Arc<Ladder>) -> (String, Result<Outcome, LadderError>) {
    answer_once(ladder, any_text()).await
}

async fn answer_once(
    ladder: &Arc<Ladder>,
    commit: Commit,
) -> (String, Result<Outcome, LadderError>) {
    let mut events = match ladder.stream(request(), commit).await {
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
