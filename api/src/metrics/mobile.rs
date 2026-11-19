use super::HANDLE;

const LINKS: &str = "sc_mobile_link_total";
const REQUESTS: &str = "sc_mobile_requests_total";
const SERVICE: &str = "api";

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
