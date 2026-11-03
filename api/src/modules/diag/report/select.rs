use backend_contracts::diag::Bundle;

use super::timeline::millis;

const MAX_TRACES: usize = 12;
const BEFORE_ERROR_MS: i64 = 60_000;

pub fn wanted(bundle: &Bundle) -> Vec<String> {
    let mut traces: Vec<String> = Vec::new();
    let mut add = |trace: &str| {
        if !traces.iter().any(|known| known == trace) {
            traces.push(trace.to_owned());
        }
    };
    for attempt in bundle.attempts.iter().rev() {
        if attempt.status == 0 || attempt.status >= 400 || attempt.error.is_some() {
            add(&attempt.trace);
        }
    }
    for error in bundle.errors.iter().rev() {
        if let Some(trace) = &error.trace {
            add(trace);
        }
        let Some(failed_at) = millis(&error.at) else {
            continue;
        };
        for attempt in bundle.attempts.iter().rev() {
            let Some(sent_at) = millis(&attempt.at) else {
                continue;
            };
            if sent_at <= failed_at && failed_at - sent_at <= BEFORE_ERROR_MS {
                add(&attempt.trace);
            }
        }
    }
    traces.truncate(MAX_TRACES);
    traces
}
