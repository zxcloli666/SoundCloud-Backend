use std::time::Duration;

use metrics_exporter_prometheus::{BuildError, Matcher, PrometheusBuilder};

use super::HANDLE;

const LINES: &str = "api_voice_speak_lines_total";
const REPLIES: &str = "api_voice_speak_replies_total";
const REPLY_DURATION: &str = "api_voice_speak_reply_duration_seconds";
const CLIP_SECONDS: &str = "api_voice_speak_clip_seconds";
const POLLS: &str = "api_voice_speak_polls_total";
const STORE_ERRORS: &str = "api_voice_speak_store_errors_total";

const SECONDS_BUCKETS: &[f64] = &[1.0, 2.0, 4.0, 8.0, 15.0, 30.0, 45.0, 60.0, 90.0, 120.0];
const TIMED: [&str; 2] = [REPLY_DURATION, CLIP_SECONDS];

pub(super) fn buckets(builder: PrometheusBuilder) -> Result<PrometheusBuilder, BuildError> {
    TIMED.iter().try_fold(builder, |builder, name| {
        builder.set_buckets_for_metric(Matcher::Full((*name).to_owned()), SECONDS_BUCKETS)
    })
}

pub fn record_speak_line(outcome: &'static str) {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::counter!(LINES, "outcome" => outcome).increment(1);
}

pub fn record_speak_reply(outcome: &'static str, elapsed: Duration, clip_seconds: Option<f64>) {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::counter!(REPLIES, "outcome" => outcome).increment(1);
    metrics::histogram!(REPLY_DURATION, "outcome" => outcome).record(elapsed.as_secs_f64());
    if let Some(seconds) = clip_seconds {
        metrics::histogram!(CLIP_SECONDS).record(seconds);
    }
}

pub fn record_speak_poll(outcome: &'static str) {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::counter!(POLLS, "outcome" => outcome).increment(1);
}

pub fn record_speak_store_error(store: &'static str, action: &'static str, error: &'static str) {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::counter!(STORE_ERRORS, "store" => store, "action" => action, "error" => error)
        .increment(1);
}
