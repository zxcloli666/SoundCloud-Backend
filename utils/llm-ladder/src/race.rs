mod run;
mod settle;

use std::collections::VecDeque;
use std::future::poll_fn;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use futures::StreamExt;
use tokio::time::{Instant, sleep_until};

use crate::error::LadderError;
use crate::ladder::{Commit, Ladder, Outcome};
use crate::observer::{Failure, Skip, StepResult};
use crate::provider::StepEvent;
use crate::request::Request;
use run::Run;
use settle::{Settled, settle};

const CHARGE_WAIT: Duration = Duration::from_millis(300);

pub(crate) enum Item {
    Delta(String),
    Finished(Outcome),
    Failed(LadderError),
}

pub(crate) struct Rule {
    pub commit: Commit,
    pub accept: Option<fn(&str) -> bool>,
}

pub(crate) struct Race {
    ladder: Arc<Ladder>,
    request: Arc<Request>,
    sent_bytes: usize,
    waiting: VecDeque<usize>,
    running: Vec<Run>,
    winner: Option<Run>,
    rule: Rule,
    collected: String,
    started: Instant,
    deadline: Instant,
    hedge_at: Option<Instant>,
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
        rule: Rule,
        started: Instant,
    ) -> Self {
        let allowed = request
            .deadline
            .map_or(ladder.deadline, |deadline| deadline.min(ladder.deadline));
        Self {
            ladder,
            sent_bytes: request.size(),
            request: Arc::new(request),
            waiting: steps.into(),
            running: Vec::new(),
            winner: None,
            rule,
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
            Signal::Step(index, Some(Ok(StepEvent::Text(text)))) => self.hear(index, &text),
            Signal::Step(index, Some(Ok(StepEvent::Usage(usage)))) => {
                self.running[index].count(usage);
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
                winner.read(&text);
                let visible = std::mem::take(&mut winner.held);
                match visible.is_empty() {
                    true => None,
                    false => self.deliver(visible),
                }
            }
            Some(Some(Ok(StepEvent::Usage(usage)))) => {
                winner.count(usage);
                None
            }
            Some(None) => self.conclude().await,
            Some(Some(Err(_))) | None if answered => self.conclude().await,
            Some(Some(Err(failure))) => self.dethrone(failure),
            None if Instant::now() >= self.deadline => Some(Item::Failed(LadderError::Deadline)),
            None => self.dethrone(Failure::Timeout),
        }
    }

    fn hear(&mut self, index: usize, text: &str) -> Option<Item> {
        let run = &mut self.running[index];
        run.read(text);
        if run.held.is_empty() {
            return None;
        }
        self.hedge_at = None;
        if (self.rule.commit)(&run.held) {
            let held = self.crown(index);
            return self.deliver(held);
        }
        if run.document.is_complete() {
            self.drop_out(index, Failure::Invalid);
        }
        None
    }

    fn start_next(&mut self) -> bool {
        while let Some(&index) = self.waiting.front() {
            let step = &self.ladder.steps[index];
            if !step.provider.hedged && !self.running.is_empty() {
                self.hedge_at = None;
                return false;
            }
            self.waiting.pop_front();
            let now = Instant::now();
            if !step.breaker.enter(now) {
                self.ladder.skipped(step, Skip::Breaker);
                continue;
            }
            let time_left = self.deadline.saturating_duration_since(now);
            let stream = step.provider.start(self.request.clone(), time_left);
            let expires = now + step.provider.total_timeout;
            self.running.push(Run::new(index, stream, now, expires));
            self.hedge_at = self.hedge_after(now);
            return true;
        }
        self.hedge_at = None;
        false
    }

    fn hedge_after(&self, now: Instant) -> Option<Instant> {
        let next = *self.waiting.front()?;
        let hedged = self.ladder.steps[next].provider.hedged;
        hedged.then(|| now + self.ladder.hedge_after)
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
            self.blame_the_silent(now);
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

    fn blame_the_silent(&mut self, now: Instant) {
        let patience = self.ladder.hedge_after;
        let (silent, speaking): (Vec<Run>, Vec<Run>) = std::mem::take(&mut self.running)
            .into_iter()
            .partition(|run| run.first_output.is_none() && now - run.started > patience);
        self.running = speaking;
        for run in silent {
            self.settle(run, StepResult::Failed(Failure::Timeout));
        }
    }

    fn crown(&mut self, index: usize) -> String {
        let mut winner = self.running.remove(index);
        for loser in std::mem::take(&mut self.running).into_iter().rev() {
            if self.rule.accept.is_some() {
                self.waiting.push_front(loser.step);
            }
            self.settle(loser, StepResult::Lost);
        }
        self.hedge_at = None;
        let held = std::mem::take(&mut winner.held);
        self.winner = Some(winner);
        held
    }

    fn deliver(&mut self, visible: String) -> Option<Item> {
        match self.rule.accept {
            Some(_) => {
                self.collected.push_str(&visible);
                None
            }
            None => Some(Item::Delta(visible)),
        }
    }

    fn drop_out(&mut self, index: usize, failure: Failure) {
        let run = self.running.remove(index);
        self.settle(run, StepResult::Failed(failure));
        self.start_next();
    }

    fn dethrone(&mut self, failure: Failure) -> Option<Item> {
        let winner = self.winner.take()?;
        self.settle(winner, StepResult::Failed(failure));
        match self.rule.accept {
            Some(_) => {
                self.collected.clear();
                None
            }
            None => Some(Item::Failed(LadderError::ProviderFailed)),
        }
    }

    async fn conclude(&mut self) -> Option<Item> {
        let winner = self.winner.as_ref()?;
        let accepted = self
            .rule
            .accept
            .is_none_or(|accept| accept(&self.collected));
        if !winner.document.is_complete() || !accepted {
            return self.dethrone(Failure::Invalid);
        }
        let winner = self.winner.take()?;
        let provider = self.ladder.steps[winner.step].provider.id.clone();
        let first_output = winner.first_output.unwrap_or_default();
        let settled = self.settle(winner, StepResult::Won);
        if let Some(charge) = settled.charge {
            let _ = tokio::time::timeout(CHARGE_WAIT, charge).await;
        }
        Some(Item::Finished(Outcome {
            provider,
            usage: settled.usage,
            basis: settled.basis,
            cost_micro_usd: settled.cost_micro_usd,
            first_output,
            elapsed: self.started.elapsed(),
        }))
    }

    fn settle(&self, run: Run, result: StepResult) -> Settled {
        settle(&self.ladder, &self.request, self.sent_bytes, run, result)
    }
}

impl Drop for Race {
    fn drop(&mut self) {
        let unfinished: Vec<Run> = self.running.drain(..).chain(self.winner.take()).collect();
        for run in unfinished {
            self.settle(run, StepResult::Cancelled);
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
