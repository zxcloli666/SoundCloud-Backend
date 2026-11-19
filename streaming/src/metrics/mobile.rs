use super::{HANDLE, SOURCE_TOTAL};

const REQUESTS: &str = "sc_mobile_requests_total";
const SERVICE: &str = "streaming";
const SOURCE: &str = "mobile";
pub(crate) const OUTCOMES: &[&str] =
    &["served", "missed", "preview", "refused", "captcha", "error"];
pub(crate) const OPS: &[&str] = &["transcodings", "media"];
const STATUS_CLASSES: &[&str] = &["2xx", "4xx", "5xx", "captcha", "transport"];

pub(super) fn register() {
    for outcome in OUTCOMES {
        metrics::counter!(SOURCE_TOTAL, "source" => SOURCE, "outcome" => *outcome).increment(0);
    }
    for op in OPS {
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

#[cfg(test)]
mod tests {
    use super::super::{HANDLE, init};

    #[test]
    fn mobile_counters_are_scraped_before_the_first_linked_stream() {
        init();

        let Some(handle) = HANDLE.get() else {
            return;
        };
        let rendered = handle.render();

        for outcome in super::OUTCOMES {
            assert!(
                rendered.contains(&format!(
                    "streaming_source_total{{source=\"mobile\",outcome=\"{outcome}\"}}"
                )),
                "{outcome}"
            );
        }
        for op in super::OPS {
            assert!(
                rendered.contains(&format!(
                    "sc_mobile_requests_total{{service=\"streaming\",op=\"{op}\",status_class=\"captcha\"}}"
                )),
                "{op}"
            );
        }
    }
}
