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
    pub basis: Basis,
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
    Request,
    Refused,
    Length,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    Breaker,
    Budget,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Basis {
    #[default]
    Reported,
    Estimated,
}

impl StepResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Won => "won",
            Self::Lost => "lost",
            Self::Cancelled => "cancelled",
            Self::Failed(Failure::Status) => "failed_status",
            Self::Failed(Failure::Transport) => "failed_transport",
            Self::Failed(Failure::Timeout) => "failed_timeout",
            Self::Failed(Failure::Request) => "failed_request",
            Self::Failed(Failure::Refused) => "failed_refused",
            Self::Failed(Failure::Length) => "failed_length",
            Self::Failed(Failure::Invalid) => "failed_invalid",
            Self::Skipped(Skip::Breaker) => "skipped_breaker",
            Self::Skipped(Skip::Budget) => "skipped_budget",
        }
    }
}

impl Failure {
    pub fn blames_provider(self) -> bool {
        matches!(self, Self::Status | Self::Transport | Self::Timeout)
    }
}

impl Basis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reported => "reported",
            Self::Estimated => "estimated",
        }
    }
}
