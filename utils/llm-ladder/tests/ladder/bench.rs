use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::NoHostSections;
use crate::vendor::{Seen, Step, Vendor};
use bytes::Bytes;
use futures::StreamExt;
use futures::future::BoxFuture;
use llm_ladder::{
    Commit, Event, Hosting, Ladder, LadderError, Ladders, Message, Outcome, Request, Schema,
    Section, Settings, System, Usage, WorkerError, WorkerTransport,
};
use serde::Deserialize;
use serde_json::Value;

pub const KEY: &str = "llm-ladder";
pub const PIECES: [&str; 3] = ["{\"say\":", "\"hello ", "there\"}"];
pub const WAIT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize, PartialEq)]
pub struct Said {
    pub say: String,
}

pub struct Bench {
    pub ladders: Ladders,
    pub seen: Arc<Seen>,
}

pub struct Answer {
    pub document: String,
    pub outcome: Result<Outcome, LadderError>,
    pub first_delta: Option<Duration>,
    pub finished: Duration,
    pub deltas: usize,
}

#[derive(Default)]
pub struct LocalModel {
    pub reply: Mutex<Option<Result<Value, WorkerError>>>,
    pub calls: Mutex<Vec<(String, Value, Duration)>>,
}

impl WorkerTransport for LocalModel {
    fn call<'a>(
        &'a self,
        subject: &'a str,
        payload: Bytes,
        timeout: Duration,
    ) -> BoxFuture<'a, Result<Value, WorkerError>> {
        Box::pin(async move {
            let payload = serde_json::from_slice(&payload).expect("the payload is JSON");
            self.calls
                .lock()
                .unwrap()
                .push((subject.to_owned(), payload, timeout));
            self.reply
                .lock()
                .unwrap()
                .clone()
                .expect("the test set a reply")
        })
    }
}

pub fn http_provider(id: &str, kind: &str, url: &str, model: &str, more: &str) -> String {
    format!(
        "[providers.{id}]\nkind = \"{kind}\"\nendpoint = \"{url}\"\nmodel = \"{model}\"\n\
         api_key = \"env:CARGO_PKG_NAME\"\nmax_output_tokens = 600\ntotal_timeout_ms = 4000\n{more}\n"
    )
}

pub fn worker_provider(id: &str, more: &str) -> String {
    format!(
        "[providers.{id}]\nkind = \"worker\"\nsubject = \"ai.rpc.complete\"\nmax_output_tokens = 400\n\
         total_timeout_ms = 5000\n{more}\n"
    )
}

pub fn ladder(id: &str, steps: &[&str], hedge_after_ms: u64) -> String {
    let steps: Vec<String> = steps.iter().map(|step| format!("\"{step}\"")).collect();
    format!(
        "[ladders.{id}]\nsteps = [{}]\nhedge_after_ms = {hedge_after_ms}\ndeadline_ms = 8000\n\
         breaker = {{ failures = 3, window_s = 30, open_s = 90 }}\n",
        steps.join(", ")
    )
}

pub fn bench(config: &str, worker: Option<Arc<dyn WorkerTransport>>) -> Bench {
    let (settings, _): (Settings, NoHostSections) =
        Settings::parse(config).expect("a valid config");
    let seen = Arc::new(Seen::default());
    let hosting = Hosting {
        jetstream: None,
        worker,
        observer: seen.clone(),
    };
    Bench {
        ladders: Ladders::build(&settings, hosting).expect("the ladders are built"),
        seen,
    }
}

pub fn request() -> Request {
    Request {
        system: System {
            sections: vec![
                Section::shared("Answer with one JSON object."),
                Section::shared("Speak as Loli."),
                Section::own("The volume is 60."),
            ],
        },
        messages: vec![
            Message::user("hi"),
            Message::assistant("hello"),
            Message::user("how are you"),
        ],
        schema: Schema::object([
            ("say", Schema::String),
            ("mood", Schema::one_of(["calm", "glad"])),
        ]),
        budget_user: None,
        deadline: None,
    }
}

pub fn any_text() -> Commit {
    Arc::new(|_| true)
}

pub async fn answer(ladder: &Arc<Ladder>) -> Answer {
    answer_to(ladder, request()).await
}

pub async fn answer_to(ladder: &Arc<Ladder>, request: Request) -> Answer {
    let started = Instant::now();
    let mut answer = Answer {
        document: String::new(),
        outcome: Err(LadderError::Unavailable),
        first_delta: None,
        finished: Duration::ZERO,
        deltas: 0,
    };
    match ladder.stream(request, any_text()).await {
        Ok(mut events) => {
            while let Some(event) = events.next().await {
                match event {
                    Ok(Event::Delta(delta)) => {
                        answer.first_delta.get_or_insert(started.elapsed());
                        answer.deltas += 1;
                        answer.document.push_str(&delta);
                    }
                    Ok(Event::Done(outcome)) => answer.outcome = Ok(outcome),
                    Err(error) => answer.outcome = Err(error),
                }
            }
        }
        Err(error) => answer.outcome = Err(error),
    }
    answer.finished = started.elapsed();
    answer
}

pub async fn answer_held_back(ladder: &Arc<Ladder>, vendor: &Vendor, early: &str) -> Answer {
    let started = Instant::now();
    let mut events = ladder
        .stream(request(), any_text())
        .await
        .expect("an open step");
    let mut answer = Answer {
        document: String::new(),
        outcome: Err(LadderError::Unavailable),
        first_delta: None,
        finished: Duration::ZERO,
        deltas: 0,
    };
    let mut released = false;
    loop {
        if !released && answer.document == early {
            vendor.release();
            released = true;
        }
        let event = tokio::time::timeout(WAIT, events.next())
            .await
            .expect("the early pieces arrive while the vendor holds the rest back");
        match event {
            Some(Ok(Event::Delta(delta))) => {
                answer.first_delta.get_or_insert(started.elapsed());
                answer.deltas += 1;
                answer.document.push_str(&delta);
            }
            Some(Ok(Event::Done(outcome))) => answer.outcome = Ok(outcome),
            Some(Err(error)) => answer.outcome = Err(error),
            None => break,
        }
    }
    assert!(
        released,
        "the vendor held the end back until {early:?} arrived"
    );
    answer.finished = started.elapsed();
    answer
}

pub fn step(provider: &str, result: &'static str, usage: (u64, u64), cost_micro_usd: u64) -> Step {
    Step {
        provider: provider.to_owned(),
        result,
        basis: "reported",
        usage: Usage {
            input_tokens: usage.0,
            output_tokens: usage.1,
            ..Usage::default()
        },
        cost_micro_usd,
    }
}

pub fn results(pairs: &[(&str, &'static str)]) -> Vec<(String, &'static str)> {
    pairs
        .iter()
        .map(|(provider, result)| ((*provider).to_owned(), *result))
        .collect()
}
