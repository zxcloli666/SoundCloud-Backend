use chrono::Utc;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use super::run::Run;
use crate::ladder::{Ladder, Step};
use crate::observer::{Basis, Failure, StepReport, StepResult};
use crate::request::{Request, Usage};

pub(super) struct Settled {
    pub usage: Usage,
    pub basis: Basis,
    pub cost_micro_usd: u64,
    pub charge: Option<JoinHandle<()>>,
}

pub(super) fn settle(
    ladder: &Ladder,
    request: &Request,
    sent_bytes: usize,
    run: Run,
    result: StepResult,
) -> Settled {
    let step = &ladder.steps[run.step];
    match result {
        StepResult::Won => step.breaker.succeeded(run.pass),
        StepResult::Failed(failure) if failure.blames_provider() => {
            step.breaker.failed(run.pass, Instant::now())
        }
        _ => step.breaker.left(run.pass),
    }
    let (usage, basis) = billed(step, &run, result, sent_bytes);
    if result == StepResult::Won && basis == Basis::Estimated {
        tracing::warn!(
            ladder = %ladder.id,
            provider = %step.provider.id,
            "a priced provider did not report the final token usage of its answer, its spend is estimated"
        );
    }
    let cost_micro_usd = step.provider.price.cost(&usage);
    ladder.observer.step(&StepReport {
        ladder: &ladder.id,
        provider: &step.provider.id,
        result,
        first_output: run.first_output,
        elapsed: run.started.elapsed(),
        usage,
        basis,
        cost_micro_usd,
    });
    let charge = ladder.budget.as_ref().and_then(|budget| {
        budget.charge(request.budget_user.as_deref(), cost_micro_usd, Utc::now())
    });
    Settled {
        usage,
        basis,
        cost_micro_usd,
        charge,
    }
}

fn billed(step: &Step, run: &Run, result: StepResult, sent_bytes: usize) -> (Usage, Basis) {
    let refused = !run.accepted
        && matches!(
            result,
            StepResult::Failed(Failure::Status | Failure::Request | Failure::Transport)
        );
    let reported = run.usage_is_final && !run.usage.is_unknown();
    if !step.provider.price.is_set() || reported || refused {
        return (run.usage, Basis::Reported);
    }
    let estimate = Usage::estimated(sent_bytes, run.received);
    (run.usage.completed_by(estimate), Basis::Estimated)
}
