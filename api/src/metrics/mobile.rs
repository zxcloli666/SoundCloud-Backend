use super::HANDLE;

const LINKS: &str = "sc_mobile_link_total";
const REQUESTS: &str = "sc_mobile_requests_total";
const SERVICE: &str = "api";
const LINK_OUTCOMES: &[&str] = &[
    "started",
    "completed",
    "account_mismatch",
    "timed_out",
    "failed",
    "unlinked",
];
const REQUEST_OPS: &[&str] = &["token", "me"];
const STATUS_CLASSES: &[&str] = &["2xx", "4xx", "5xx", "captcha", "transport"];

pub(super) fn register() {
    for outcome in LINK_OUTCOMES {
        metrics::counter!(LINKS, "outcome" => *outcome).increment(0);
    }
    for op in REQUEST_OPS {
        for class in STATUS_CLASSES {
            metrics::counter!(
                REQUESTS,
                "service" => SERVICE,
                "op" => *op,
                "status_class" => *class
            )
            .increment(0);
        }
    }
}

pub fn record_mobile_link(outcome: &'static str) {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::counter!(LINKS, "outcome" => outcome).increment(1);
}

pub fn record_mobile_request(op: &'static str, status_class: &'static str) {
    if HANDLE.get().is_none() {
        return;
    }
    metrics::counter!(
        REQUESTS,
        "service" => SERVICE,
        "op" => op,
        "status_class" => status_class
    )
    .increment(1);
}
