use std::time::Duration;

use llm_ladder::{StepReport, StepResult};
use metrics_exporter_prometheus::{BuildError, Matcher, PrometheusBuilder};

use super::HANDLE;

pub(super) const STEPS: &str = "api_llm_steps_total";
pub(super) const STEP_DURATION: &str = "api_llm_step_duration_seconds";
pub(super) const FIRST_OUTPUT: &str = "api_llm_first_output_seconds";
pub(super) const TOKENS: &str = "api_llm_tokens_total";
pub(super) const SPEND: &str = "api_llm_spend_micro_usd_total";
pub(super) const CHAT_TURNS: &str = "api_voice_chat_turns_total";
pub(super) const CHAT_DURATION: &str = "api_voice_chat_turn_duration_seconds";
pub(super) const CHAT_FIRST_EVENT: &str = "api_voice_chat_first_event_seconds";
pub(super) const CHAT_ACTIONS: &str = "api_voice_chat_actions_total";

const ANSWER_BUCKETS: &[f64] = &[
    0.25, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 20.0, 30.0,
];
const TIMED: [&str; 4] = [STEP_DURATION, FIRST_OUTPUT, CHAT_DURATION, CHAT_FIRST_EVENT];

pub(super) fn buckets(builder: PrometheusBuilder) -> Result<PrometheusBuilder, BuildError> {
    TIMED.iter().try_fold(builder, |builder, name| {
        builder.set_buckets_for_metric(Matcher::Full((*name).to_owned()), ANSWER_BUCKETS)
    })
}

pub fn record_llm_step(report: &StepReport<'_>) {
    if HANDLE.get().is_none() {
        return;
    }
    let ladder = report.ladder.to_owned();
    let provider = report.provider.to_owned();
    let result = report.result.as_str();
    metrics::counter!(
        STEPS,
        "ladder" => ladder.clone(),
        "provider" => provider.clone(),
        "result" => result,
    )
    .increment(1);
    if matches!(report.result, StepResult::Skipped(_)) {
        return;
    }
    metrics::histogram!(
        STEP_DURATION,
        "ladder" => ladder.clone(),
        "provider" => provider.clone(),
        "result" => result,
    )
    .record(report.elapsed.as_secs_f64());
    if let Some(first_output) = report.first_output {
        metrics::histogram!(
            FIRST_OUTPUT,
            "ladder" => ladder.clone(),
            "provider" => provider.clone(),
        )
        .record(first_output.as_secs_f64());
    }
    for (kind, tokens) in [
        ("input", report.usage.input_tokens),
        ("cache_read", report.usage.cache_read_tokens),
        ("cache_write", report.usage.cache_write_tokens),
        ("output", report.usage.output_tokens),
    ] {
        metrics::counter!(
            TOKENS,
            "ladder" => ladder.clone(),
            "provider" => provider.clone(),
            "kind" => kind,
        )
        .increment(tokens);
    }
    metrics::counter!(SPEND, "ladder" => ladder, "provider" => provider)
        .increment(report.cost_micro_usd);
}

pub fn record_voice_chat(
    outcome: &'static str,
    elapsed: Duration,
    first_event: Option<Duration>,
    actions: u64,
    dropped: u64,
) {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::counter!(CHAT_TURNS, "outcome" => outcome).increment(1);
    metrics::histogram!(CHAT_DURATION, "outcome" => outcome).record(elapsed.as_secs_f64());
    if let Some(first_event) = first_event {
        metrics::histogram!(CHAT_FIRST_EVENT).record(first_event.as_secs_f64());
    }
    metrics::counter!(CHAT_ACTIONS, "result" => "sent").increment(actions);
    metrics::counter!(CHAT_ACTIONS, "result" => "dropped").increment(dropped);
}
