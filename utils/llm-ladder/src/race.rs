use std::collections::VecDeque;
use std::future::poll_fn;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use chrono::Utc;
use futures::StreamExt;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep_until};

use crate::document::Document;
use crate::error::LadderError;
use crate::ladder::{Ladder, Outcome};
use crate::observer::{Failure, Skip, StepReport, StepResult};
use crate::provider::{StepEvent, StepStream};
use crate::request::{Request, Usage};

const CHARGE_WAIT: Duration = Duration::from_millis(300);
const USAGE_WAIT: Duration = Duration::from_secs(1);

pub(crate) enum Item {
    Delta(String),
    Finished(Outcome),
    Failed(LadderError),
}

pub(crate) struct Race {
    ladder: Arc<Ladder>,
    request: Arc<Request>,
    waiting: VecDeque<usize>,
    running: Vec<Run>,
    winner: Option<Run>,
    accept: Option<fn(&str) -> bool>,
    collected: String,
    started: Instant,
    deadline: Instant,
    hedge_at: Option<Instant>,
}

struct Run {
    step: usize,
    stream: StepStream,
    document: Document,
    usage: Usage,
    started: Instant,
    expires: Instant,
    first_output: Option<Duration>,
    answered_at: Option<Instant>,
}

enum Signal {
    Step(usize, Option<Result<StepEvent, Failure>>),
    Timer,
}

impl Race {
    pub fn new(
        ladder: Arc<Ladder>,
        request: Request,
        steps: Vec<usize>,
        accept: Option<fn(&str) -> bool>,
        started: Instant,
    ) -> Self {
        let allowed = request
            .deadline
            .map_or(ladder.deadline, |deadline| deadline.min(ladder.deadline));
        Self {
            ladder,
            request: Arc::new(request),
            waiting: steps.into(),
            running: Vec::new(),
            winner: None,
            accept,
            collected: String::new(),
            started,
            deadline: started + allowed,
            hedge_at: None,
        }
    }

    pub async fn next(&mut self) -> Item {
        loop {
            let item = match self.winner.is_some() {
                true => self.follow().await,
                false => self.contend().await,
            };
            if let Some(item) = item {
                return item;
            }
        }
    }

    pub fn take_collected(&mut self) -> String {
        std::mem::take(&mut self.collected)
    }

    async fn contend(&mut self) -> Option<Item> {
        if self.running.is_empty() && !self.start_next() {
            return Some(Item::Failed(LadderError::Exhausted));
        }
        let wake = self.wake_at();
        let signal = tokio::select! {
            biased;
            (index, event) = next_event(&mut self.running) => Signal::Step(index, event),
            _ = sleep_until(wake) => Signal::Timer,
        };
        match signal {
            Signal::Step(index, Some(Ok(StepEvent::Text(text)))) => {
                let visible = self.running[index].read(&text);
                if visible.is_empty() {
                    return None;
                }
                self.crown(index);
                self.deliver(visible)
            }
            Signal::Step(index, Some(Ok(StepEvent::Usage(usage)))) => {
                self.running[index].usage = usage;
                None
            }
            Signal::Step(index, Some(Err(failure))) => {
                self.drop_out(index, failure);
                None
            }
            Signal::Step(index, None) => {
                self.drop_out(index, Failure::Invalid);
                None
            }
            Signal::Timer => self.on_time(),
        }
    }

    async fn follow(&mut self) -> Option<Item> {
        let winner = self.winner.as_mut()?;
        let answered = winner.document.is_complete();
        let wake = winner.wake_at(self.deadline);
        let event = tokio::select! {
            biased;
            event = winner.stream.next() => Some(event),
            _ = sleep_until(wake) => None,
        };
        match event {
            Some(Some(Ok(StepEvent::Text(text)))) => {
                let visible = winner.read(&text);
                match visible.is_empty() {
                    true => None,
                    false => self.deliver(visible),
                }
            }
            Some(Some(Ok(StepEvent::Usage(usage)))) => {
                winner.usage = usage;
                None
            }
            Some(None) => self.conclude().await,
            Some(Some(Err(_))) | None if answered => self.conclude().await,
            Some(Some(Err(failure))) => self.dethrone(failure),
            None if Instant::now() >= self.deadline => Some(Item::Failed(LadderError::Deadline)),
            None => self.dethrone(Failure::Timeout),
        }
    }

    fn start_next(&mut self) -> bool {
        while let Some(index) = self.waiting.pop_front() {
            let now = Instant::now();
            let step = &self.ladder.steps[index];
            if !step.breaker.enter(now) {
                self.ladder.skipped(step, Skip::Breaker);
                continue;
            }
            self.running.push(Run {
                step: index,
                stream: step.provider.start(self.request.clone()),
                document: Document::default(),
                usage: Usage::default(),
                started: now,
                expires: now + step.provider.total_timeout,
                first_output: None,
                answered_at: None,
            });
            self.hedge_at = (!self.waiting.is_empty()).then(|| now + self.ladder.hedge_after);
            return true;
        }
        self.hedge_at = None;
        false
    }

    fn wake_at(&self) -> Instant {
        self.running
            .iter()
            .map(|run| run.expires)
            .chain(self.hedge_at)
            .fold(self.deadline, Instant::min)
    }

    fn on_time(&mut self) -> Option<Item> {
        let now = Instant::now();
        if now >= self.deadline {
            return Some(Item::Failed(LadderError::Deadline));
        }
        while let Some(index) = self.running.iter().position(|run| run.expires <= now) {
            self.drop_out(index, Failure::Timeout);
        }
        if self.hedge_at.is_some_and(|at| at <= now) {
            self.start_next();
        }
        None
    }

    fn crown(&mut self, index: usize) {
        let mut winner = self.running.remove(index);
        winner.first_output = Some(winner.started.elapsed());
        for loser in std::mem::take(&mut self.running).into_iter().rev() {
            if self.accept.is_some() {
                self.waiting.push_front(loser.step);
            }
            settle(&self.ladder, &self.request, loser, StepResult::Lost);
        }
        self.hedge_at = None;
        self.winner = Some(winner);
    }

    fn deliver(&mut self, visible: String) -> Option<Item> {
        match self.accept {
            Some(_) => {
                self.collected.push_str(&visible);
                None
            }
            None => Some(Item::Delta(visible)),
        }
    }

    fn drop_out(&mut self, index: usize, failure: Failure) {
        let run = self.running.remove(index);
        settle(
            &self.ladder,
            &self.request,
            run,
            StepResult::Failed(failure),
        );
        self.start_next();
    }

    fn dethrone(&mut self, failure: Failure) -> Option<Item> {
        let winner = self.winner.take()?;
        settle(
            &self.ladder,
            &self.request,
            winner,
            StepResult::Failed(failure),
        );
        match self.accept {
            Some(_) => {
                self.collected.clear();
                None
            }
            None => Some(Item::Failed(LadderError::ProviderFailed)),
        }
    }

    async fn conclude(&mut self) -> Option<Item> {
        let winner = self.winner.as_ref()?;
        let accepted = self.accept.is_none_or(|accept| accept(&self.collected));
        if !winner.document.is_complete() || !accepted {
            return self.dethrone(Failure::Invalid);
        }
        let winner = self.winner.take()?;
        let provider = &self.ladder.steps[winner.step].provider;
        let outcome = Outcome {
            provider: provider.id.clone(),
            usage: winner.usage,
            cost_micro_usd: provider.price.cost(&winner.usage),
            first_output: winner.first_output.unwrap_or_default(),
            elapsed: self.started.elapsed(),
        };
        if let Some(charge) = settle(&self.ladder, &self.request, winner, StepResult::Won) {
            let _ = tokio::time::timeout(CHARGE_WAIT, charge).await;
        }
        Some(Item::Finished(outcome))
    }
}

impl Run {
    fn read(&mut self, text: &str) -> String {
        let visible = self.document.push(text);
        if self.document.is_complete() {
            self.answered_at.get_or_insert_with(Instant::now);
        }
        visible
    }

    fn wake_at(&self, deadline: Instant) -> Instant {
        let tail = self.answered_at.map_or(deadline, |at| at + USAGE_WAIT);
        self.expires.min(deadline).min(tail)
    }
}

impl Drop for Race {
    fn drop(&mut self) {
        let unfinished: Vec<Run> = self.running.drain(..).chain(self.winner.take()).collect();
        for run in unfinished {
            settle(&self.ladder, &self.request, run, StepResult::Cancelled);
        }
    }
}

async fn next_event(running: &mut [Run]) -> (usize, Option<Result<StepEvent, Failure>>) {
    poll_fn(|context| {
        for (index, run) in running.iter_mut().enumerate() {
            if let Poll::Ready(event) = run.stream.poll_next_unpin(context) {
                return Poll::Ready((index, event));
            }
        }
        Poll::Pending
    })
    .await
}

fn settle(
    ladder: &Ladder,
    request: &Request,
    run: Run,
    result: StepResult,
) -> Option<JoinHandle<()>> {
    let step = &ladder.steps[run.step];
    match result {
        StepResult::Won => step.breaker.succeeded(),
        StepResult::Failed(_) => step.breaker.failed(Instant::now()),
        StepResult::Lost | StepResult::Cancelled | StepResult::Skipped(_) => step.breaker.left(),
    }
    let priced = step.provider.price.is_set();
    if result == StepResult::Won && priced && run.usage.is_unknown() {
        tracing::warn!(
            ladder = %ladder.id,
            provider = %step.provider.id,
            "a priced provider reported no token usage, its spend is not counted"
        );
    }
    let cost_micro_usd = step.provider.price.cost(&run.usage);
    ladder.observer.step(&StepReport {
        ladder: &ladder.id,
        provider: &step.provider.id,
        result,
        first_output: run.first_output,
        elapsed: run.started.elapsed(),
        usage: run.usage,
        cost_micro_usd,
    });
    ladder
        .budget
        .as_ref()?
        .charge(request.budget_user.as_deref(), cost_micro_usd, Utc::now())
}
