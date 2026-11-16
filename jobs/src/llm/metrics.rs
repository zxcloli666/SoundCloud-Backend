use llm_ladder::{Observer, StepReport, StepResult};
use metrics_exporter_prometheus::{BuildError, Matcher, PrometheusBuilder};

const STEPS: &str = "jobs_llm_steps_total";
const STEP_DURATION: &str = "jobs_llm_step_duration_seconds";
const FIRST_OUTPUT: &str = "jobs_llm_first_output_seconds";
const TOKENS: &str = "jobs_llm_tokens_total";
const SPEND: &str = "jobs_llm_spend_micro_usd_total";
const ANSWERS: &str = "jobs_ai_answers_total";

const ANSWER_BUCKETS: &[f64] = &[
    0.25, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 20.0, 30.0,
];
const TIMED: [&str; 2] = [STEP_DURATION, FIRST_OUTPUT];

pub struct LadderMetrics;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiMethod {
    ResolveArtist,
    MatchTrack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiOutcome {
    Confident,
    Cached,
    Llm,
    Fallback,
    OverBudget,
}

impl Observer for LadderMetrics {
    fn step(&self, report: &StepReport<'_>) {
        record_step(report);
    }
}

impl AiMethod {
    const ALL: [Self; 2] = [Self::ResolveArtist, Self::MatchTrack];

    fn as_str(self) -> &'static str {
        match self {
            Self::ResolveArtist => "resolve_artist",
            Self::MatchTrack => "match_track",
        }
    }
}

impl AiOutcome {
    const ALL: [Self; 5] = [
        Self::Confident,
        Self::Cached,
        Self::Llm,
        Self::Fallback,
        Self::OverBudget,
    ];

    fn as_str(self) -> &'static str {
        match self {
            Self::Confident => "confident",
            Self::Cached => "cached",
            Self::Llm => "llm",
            Self::Fallback => "fallback",
            Self::OverBudget => "over_budget",
        }
    }
}

pub fn buckets(builder: PrometheusBuilder) -> Result<PrometheusBuilder, BuildError> {
    TIMED.iter().try_fold(builder, |builder, name| {
        builder.set_buckets_for_metric(Matcher::Full((*name).to_owned()), ANSWER_BUCKETS)
    })
}

pub fn start_at_zero() {
    for method in AiMethod::ALL {
        for outcome in AiOutcome::ALL {
            metrics::counter!(ANSWERS, "method" => method.as_str(), "outcome" => outcome.as_str())
                .increment(0);
        }
    }
}

pub fn record_answer(method: AiMethod, outcome: AiOutcome) {
    metrics::counter!(ANSWERS, "method" => method.as_str(), "outcome" => outcome.as_str())
        .increment(1);
}

fn record_step(report: &StepReport<'_>) {
    let ladder = report.ladder.to_owned();
    let provider = report.provider.to_owned();
    let result = report.result.as_str();
    let basis = report.basis.as_str();
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
            "basis" => basis,
        )
        .increment(tokens);
    }
    metrics::counter!(
        SPEND,
        "ladder" => ladder,
        "provider" => provider,
        "basis" => basis,
    )
    .increment(report.cost_micro_usd);
}
