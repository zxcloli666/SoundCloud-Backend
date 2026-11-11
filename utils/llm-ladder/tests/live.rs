use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_nats::jetstream;
use futures::StreamExt;
use llm_ladder::{
    BudgetSettings, CapSettings, Event, Hosting, Kind, Ladder, LadderSettings, Ladders, Message,
    Observer, Outcome, Request, Schema, Settings, StepReport,
};
use serde::Deserialize;
use serde_json::Value;

const SYSTEM: &str = "You are the voice of a music player. Answer with exactly one JSON object and nothing else: \
{\"actions\":[{\"name\":\"<tool>\",\"args\":[{\"name\":\"<argument>\",\"value\":<string, number or boolean>}]}],\"say\":\"<one short spoken sentence>\"}. \
Tools: volume_set(level: integer 0..100) sets the playback volume; next_track() skips to the next track; \
like_track() likes the current track. Use an empty actions list when no tool fits. Current volume: 40.";
const PHRASE: &str = "Make it a bit louder and tell me something nice.";
const RACES: usize = 3;

#[derive(Debug, Deserialize)]
struct Answer {
    actions: Vec<Action>,
    say: String,
}

#[derive(Debug, Deserialize)]
struct Action {
    name: String,
    #[serde(default)]
    args: Value,
}

#[derive(Default)]
struct Printed(Mutex<Vec<String>>);

struct Measured {
    answer: Answer,
    outcome: Outcome,
    first_delta: Duration,
    total: Duration,
    deltas: usize,
}

impl Observer for Printed {
    fn step(&self, report: &StepReport<'_>) {
        self.0.lock().unwrap().push(format!(
            "{}={} first_output_ms={} elapsed_ms={} in={} out={} cost_micro_usd={}",
            report.provider,
            report.result.as_str(),
            report.first_output.map_or(-1, |at| at.as_millis() as i64),
            report.elapsed.as_millis(),
            report.usage.input_tokens,
            report.usage.output_tokens,
            report.cost_micro_usd,
        ));
    }
}

impl Printed {
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.0.lock().unwrap())
    }
}

fn settings() -> Settings {
    let path = std::env::var("LLM_CONFIG").expect("LLM_CONFIG names the config file");
    let (settings, _): (Settings, toml::Table) =
        Settings::load(Path::new(&path)).expect("a valid config");
    settings
}

fn live_ladder(settings: &Settings) -> (String, LadderSettings) {
    let id =
        std::env::var("LLM_LIVE_LADDER").expect("LLM_LIVE_LADDER names a ladder of the config");
    let ladder = settings
        .ladders
        .get(&id)
        .expect("the ladder exists")
        .clone();
    (id, ladder)
}

fn schema() -> Schema {
    let argument = Schema::object([
        ("name", Schema::String),
        (
            "value",
            Schema::AnyOf(vec![Schema::String, Schema::Number, Schema::Boolean]),
        ),
    ]);
    let action = Schema::object([
        (
            "name",
            Schema::one_of(["volume_set", "next_track", "like_track"]),
        ),
        ("args", Schema::array(argument)),
    ]);
    Schema::object([("actions", Schema::array(action)), ("say", Schema::String)])
}

fn request(user: Option<&str>) -> Request {
    Request {
        system: SYSTEM.to_owned(),
        messages: vec![Message::user(PHRASE)],
        schema: schema(),
        budget_user: user.map(str::to_owned),
        deadline: None,
    }
}

async fn measure(ladder: &Arc<Ladder>, user: Option<&str>) -> Measured {
    let started = Instant::now();
    let mut events = ladder.stream(request(user)).await.expect("a step is open");
    let mut document = String::new();
    let mut first_delta = None;
    let mut deltas = 0;
    loop {
        match events
            .next()
            .await
            .expect("the stream ends with a final item")
        {
            Ok(Event::Delta(delta)) => {
                first_delta.get_or_insert(started.elapsed());
                deltas += 1;
                document.push_str(&delta);
            }
            Ok(Event::Done(outcome)) => {
                let answer: Answer =
                    serde_json::from_str(&document).expect("the answer fits the shape");
                assert!(!answer.say.trim().is_empty(), "the answer says something");
                return Measured {
                    answer,
                    outcome,
                    first_delta: first_delta.expect("a delta came before the end"),
                    total: started.elapsed(),
                    deltas,
                };
            }
            Err(error) => panic!("the ladder failed: {error:?}"),
        }
    }
}

fn report(label: &str, measured: &Measured) {
    println!(
        "{label} winner={} first_delta_ms={} total_ms={} deltas={} in={} out={} cost_micro_usd={} actions={:?} say_chars={}",
        measured.outcome.provider,
        measured.first_delta.as_millis(),
        measured.total.as_millis(),
        measured.deltas,
        measured.outcome.usage.input_tokens,
        measured.outcome.usage.output_tokens,
        measured.outcome.cost_micro_usd,
        measured
            .answer
            .actions
            .iter()
            .map(|action| format!("{}{}", action.name, action.args))
            .collect::<Vec<_>>(),
        measured.answer.say.chars().count(),
    );
}

fn build(settings: &Settings, jetstream: Option<jetstream::Context>) -> (Ladders, Arc<Printed>) {
    let printed = Arc::new(Printed::default());
    let hosting = Hosting {
        jetstream,
        worker: None,
        observer: printed.clone(),
    };
    (
        Ladders::build(settings, hosting).expect("the ladders are built"),
        printed,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires real providers: LLM_CONFIG and the keys it names"]
async fn every_http_provider_of_the_config_answers_as_a_stream_and_in_one_shot() {
    let source = settings();
    let mut solo = Settings::default();
    let template = source
        .ladders
        .values()
        .next()
        .expect("the config has a ladder")
        .clone();
    for (id, provider) in source
        .providers
        .iter()
        .filter(|(_, provider)| provider.kind != Kind::Worker)
    {
        for (mode, stream) in [("stream", true), ("whole", false)] {
            let name = format!("{id}-{mode}");
            let mut provider = provider.clone();
            provider.stream = Some(stream);
            solo.providers.insert(name.clone(), provider);
            let ladder = LadderSettings {
                steps: vec![name.clone()],
                budget: CapSettings::default(),
                ..template.clone()
            };
            solo.ladders.insert(name, ladder);
        }
    }
    let (ladders, printed) = build(&solo, None);

    for id in solo.ladders.keys() {
        let measured = measure(&ladders.get(id).unwrap(), None).await;
        report(id, &measured);
        printed.take();
        if id.ends_with("-whole") {
            assert_eq!(measured.deltas, 1, "a one-shot answer is one delta");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires real providers: LLM_CONFIG, LLM_LIVE_LADDER and the keys the config names"]
async fn a_ladder_of_the_config_races_its_steps_when_the_hedge_is_immediate() {
    let mut settings = settings();
    let (id, mut ladder) = live_ladder(&settings);
    ladder.hedge_after_ms = 1;
    ladder.budget = CapSettings::default();
    settings.ladders.insert(id.clone(), ladder);
    settings.budget = None;
    let (ladders, printed) = build(&settings, None);
    let ladder = ladders.get(&id).unwrap();

    for race in 0..RACES {
        let measured = measure(&ladder, None).await;
        report(&format!("race{race}"), &measured);
        let steps = printed.take();
        println!("race{race} steps: {}", steps.join(" | "));
        assert!(steps.len() >= 2, "more than one step ran: {steps:?}");
        assert_eq!(
            steps.iter().filter(|step| step.contains("=won ")).count(),
            1
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires real providers and NATS: LLM_CONFIG, LLM_LIVE_LADDER, NATS_URL and the keys the config names"]
async fn a_spent_cap_moves_the_next_request_to_the_unpriced_steps() {
    let mut settings = settings();
    let (id, mut ladder) = live_ladder(&settings);
    ladder.hedge_after_ms = ladder.deadline_ms;
    ladder.budget = CapSettings {
        user_day_usd: Some(0.000_001),
        ..CapSettings::default()
    };
    settings.ladders.insert(id.clone(), ladder);
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();
    let bucket = format!("spend_live_{micros}");
    settings.budget = Some(BudgetSettings {
        bucket: bucket.clone(),
    });
    let url = std::env::var("NATS_URL").expect("NATS_URL");
    let context = jetstream::new(async_nats::connect(url).await.expect("nats"));
    let (ladders, printed) = build(&settings, Some(context.clone()));
    let ladder = ladders.get(&id).unwrap();

    let priced = measure(&ladder, Some("live-user")).await;
    report("before-cap", &priced);
    println!("before-cap steps: {}", printed.take().join(" | "));
    assert!(
        priced.outcome.cost_micro_usd > 0,
        "the first answer came from a priced step"
    );

    let store = context.get_key_value(&bucket).await.unwrap();
    let limit = Instant::now() + Duration::from_secs(10);
    let counted = loop {
        let mut keys = store.keys().await.unwrap();
        let key = keys.next().await.and_then(Result::ok);
        if let Some(key) = key {
            let value = store.get(key.as_str()).await.unwrap().unwrap();
            break (key, String::from_utf8(value.to_vec()).unwrap());
        }
        assert!(Instant::now() < limit, "the spend was counted");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    println!(
        "counter key_shape={} value_micro_usd={}",
        counted.0.replace(char::is_numeric, "N"),
        counted.1
    );
    assert_eq!(counted.1, priced.outcome.cost_micro_usd.to_string());

    let free = measure(&ladder, Some("live-user")).await;
    report("after-cap", &free);
    let steps = printed.take();
    println!("after-cap steps: {}", steps.join(" | "));
    assert_eq!(free.outcome.cost_micro_usd, 0);
    assert!(
        steps.iter().any(|step| step.contains("=skipped_budget ")),
        "{steps:?}"
    );

    context.delete_key_value(&bucket).await.unwrap();
}
