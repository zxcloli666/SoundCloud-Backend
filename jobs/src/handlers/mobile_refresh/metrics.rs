use sc_mobile::{MobileError, TokenSet};

const REQUESTS: &str = "sc_mobile_requests_total";

pub(super) fn refresh(answer: &Result<TokenSet, MobileError>) {
    let status_class = match answer {
        Ok(_) => "2xx",
        Err(error) => error.status_class(),
    };
    metrics::counter!(
        REQUESTS,
        "service" => "jobs",
        "op" => "refresh",
        "status_class" => status_class
    )
    .increment(1);
}
