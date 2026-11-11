use std::time::Duration;

use crate::request::Usage;

pub trait Observer: Send + Sync {
    fn step(&self, report: &StepReport<'_>);
}

#[derive(Clone, Copy, Debug)]
pub struct StepReport<'a> {
    pub ladder: &'a str,
    pub provider: &'a str,
    pub result: StepResult,
    pub first_output: Option<Duration>,
    pub elapsed: Duration,
    pub usage: Usage,
    pub cost_micro_usd: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepResult {
    Won,
    Lost,
    Cancelled,
    Failed(Failure),
    Skipped(Skip),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Status,
    Transport,
    Timeout,
    Invalid,
    Refused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    Breaker,
    Budget,
}

pub struct Silent;

impl StepResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Won => "won",
            Self::Lost => "lost",
            Self::Cancelled => "cancelled",
            Self::Failed(Failure::Status) => "failed_status",
            Self::Failed(Failure::Transport) => "failed_transport",
            Self::Failed(Failure::Timeout) => "failed_timeout",
            Self::Failed(Failure::Invalid) => "failed_invalid",
            Self::Failed(Failure::Refused) => "failed_refused",
            Self::Skipped(Skip::Breaker) => "skipped_breaker",
            Self::Skipped(Skip::Budget) => "skipped_budget",
        }
    }
}

impl Observer for Silent {
    fn step(&self, _report: &StepReport<'_>) {}
}
