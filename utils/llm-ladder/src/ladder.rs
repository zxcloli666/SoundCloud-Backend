use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use chrono::Utc;
use futures::stream::{self, BoxStream, Stream, StreamExt};
use serde::de::DeserializeOwned;
use tokio::time::Instant;

use crate::breaker::Breaker;
use crate::budget::{Budget, Caps, NatsSpend, SpendStore};
use crate::config::{LadderSettings, Settings};
use crate::error::{ConfigError, LadderError};
use crate::observer::{Observer, Skip, StepReport, StepResult};
use crate::provider::worker::WorkerTransport;
use crate::provider::{self, Provider};
use crate::race::{Item, Race};
use crate::request::{Request, Usage};

pub struct Ladders {
    ladders: HashMap<String, Arc<Ladder>>,
}

pub struct Hosting {
    pub jetstream: Option<async_nats::jetstream::Context>,
    pub worker: Option<Arc<dyn WorkerTransport>>,
    pub observer: Arc<dyn Observer>,
}

pub struct Ladder {
    pub(crate) id: String,
    pub(crate) steps: Vec<Step>,
    pub(crate) hedge_after: Duration,
    pub(crate) deadline: Duration,
    pub(crate) budget: Option<Arc<Budget>>,
    pub(crate) observer: Arc<dyn Observer>,
}

pub(crate) struct Step {
    pub provider: Arc<Provider>,
    pub breaker: Breaker,
}

pub struct AnswerStream {
    events: BoxStream<'static, Result<Event, LadderError>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Delta(String),
    Done(Outcome),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub provider: String,
    pub usage: Usage,
    pub cost_micro_usd: u64,
    pub first_output: Duration,
    pub elapsed: Duration,
}

#[derive(Clone, Debug)]
pub struct Completion<T> {
    pub value: T,
    pub outcome: Outcome,
}

impl Ladders {
    pub fn build(settings: &Settings, hosting: Hosting) -> Result<Self, ConfigError> {
        let http = provider::client()
            .map_err(|error| ConfigError::Invalid(format!("http client: {error}")))?;
        let spend = spend_store(settings, hosting.jetstream)?;
        let mut providers = HashMap::new();
        for (id, provider) in &settings.providers {
            match Provider::build(id, provider, &http, hosting.worker.as_ref()) {
                Some(provider) => {
                    providers.insert(id.as_str(), Arc::new(provider));
                }
                None => {
                    tracing::warn!(provider = %id, "llm provider is off: its secret or transport is missing")
                }
            }
        }
        let ladders = settings
            .ladders
            .iter()
            .map(|(id, ladder)| {
                let ladder = Ladder::assemble(
                    id,
                    ladder,
                    &providers,
                    spend.clone(),
                    hosting.observer.clone(),
                );
                (id.clone(), Arc::new(ladder))
            })
            .collect();
        Ok(Self { ladders })
    }

    pub fn get(&self, id: &str) -> Option<Arc<Ladder>> {
        self.ladders.get(id).cloned()
    }
}

impl Ladder {
    pub async fn stream(self: &Arc<Self>, request: Request) -> Result<AnswerStream, LadderError> {
        let race = self.enter(request, None).await?;
        let events = stream::unfold(Some(race), |race| async move {
            let mut race = race?;
            match race.next().await {
                Item::Delta(text) => Some((Ok(Event::Delta(text)), Some(race))),
                Item::Finished(outcome) => Some((Ok(Event::Done(outcome)), None)),
                Item::Failed(error) => Some((Err(error), None)),
            }
        });
        Ok(AnswerStream {
            events: events.boxed(),
        })
    }

    pub async fn complete<T: DeserializeOwned>(
        self: &Arc<Self>,
        request: Request,
    ) -> Result<Completion<T>, LadderError> {
        let mut race = self.enter(request, Some(parses::<T>)).await?;
        loop {
            match race.next().await {
                Item::Delta(_) => {}
                Item::Finished(outcome) => {
                    let value = serde_json::from_str(&race.take_collected())
                        .map_err(|_| LadderError::ProviderFailed)?;
                    return Ok(Completion { value, outcome });
                }
                Item::Failed(error) => return Err(error),
            }
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn assemble(
        id: &str,
        settings: &LadderSettings,
        providers: &HashMap<&str, Arc<Provider>>,
        spend: Option<Arc<dyn SpendStore>>,
        observer: Arc<dyn Observer>,
    ) -> Self {
        let caps = Caps::from(settings.budget);
        let steps = settings
            .steps
            .iter()
            .filter_map(|step| providers.get(step.as_str()))
            .map(|provider| Step {
                provider: provider.clone(),
                breaker: Breaker::new(settings.breaker),
            })
            .collect();
        Self {
            id: id.to_owned(),
            steps,
            hedge_after: Duration::from_millis(settings.hedge_after_ms),
            deadline: Duration::from_millis(settings.deadline_ms),
            budget: spend
                .filter(|_| caps.is_set())
                .map(|store| Arc::new(Budget::new(store, id, caps))),
            observer,
        }
    }

    pub(crate) fn skipped(&self, step: &Step, reason: Skip) {
        self.observer.step(&StepReport {
            ladder: &self.id,
            provider: &step.provider.id,
            result: StepResult::Skipped(reason),
            first_output: None,
            elapsed: Duration::ZERO,
            usage: Usage::default(),
            cost_micro_usd: 0,
        });
    }

    async fn enter(
        self: &Arc<Self>,
        request: Request,
        accept: Option<fn(&str) -> bool>,
    ) -> Result<Race, LadderError> {
        let started = Instant::now();
        let steps = self.open_steps(&request).await;
        if steps.is_empty() {
            return Err(LadderError::Unavailable);
        }
        Ok(Race::new(self.clone(), request, steps, accept, started))
    }

    async fn open_steps(&self, request: &Request) -> Vec<usize> {
        let mut steps: Vec<usize> = (0..self.steps.len()).collect();
        let priced = |step: &usize| self.steps[*step].provider.price.is_set();
        if let Some(budget) = &self.budget
            && steps.iter().any(priced)
            && budget
                .is_spent(request.budget_user.as_deref(), Utc::now())
                .await
        {
            for step in steps.iter().filter(|step| priced(step)) {
                self.skipped(&self.steps[*step], Skip::Budget);
            }
            steps.retain(|step| !priced(step));
        }
        let now = Instant::now();
        steps.retain(|step| {
            let open = self.steps[*step].breaker.is_open(now);
            if open {
                self.skipped(&self.steps[*step], Skip::Breaker);
            }
            !open
        });
        steps
    }
}

impl Stream for AnswerStream {
    type Item = Result<Event, LadderError>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.events.poll_next_unpin(context)
    }
}

fn spend_store(
    settings: &Settings,
    jetstream: Option<async_nats::jetstream::Context>,
) -> Result<Option<Arc<dyn SpendStore>>, ConfigError> {
    match (&settings.budget, jetstream) {
        (None, _) => Ok(None),
        (Some(budget), Some(jetstream)) => {
            Ok(Some(Arc::new(NatsSpend::new(jetstream, &budget.bucket))))
        }
        (Some(_), None) => Err(ConfigError::Invalid(
            "[budget] needs a JetStream connection from the host service".to_owned(),
        )),
    }
}

fn parses<T: DeserializeOwned>(document: &str) -> bool {
    serde_json::from_str::<T>(document).is_ok()
}

#[cfg(test)]
#[path = "ladder_tests.rs"]
mod tests;
