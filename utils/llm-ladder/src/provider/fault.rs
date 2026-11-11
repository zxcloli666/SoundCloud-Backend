use serde_json::Value;

use crate::observer::Failure;

const REQUEST_STATUSES: [u64; 3] = [400, 413, 422];
const REQUEST_KINDS: [&str; 3] = [
    "invalid_request_error",
    "request_too_large",
    "context_length_exceeded",
];

pub(crate) fn of_status(status: u16) -> Failure {
    match REQUEST_STATUSES.contains(&u64::from(status)) {
        true => Failure::Request,
        false => Failure::Status,
    }
}

pub(crate) fn of_error(problem: &Value) -> Failure {
    let named = ["type", "code"]
        .iter()
        .filter_map(|field| problem.get(field).and_then(Value::as_str))
        .any(|kind| REQUEST_KINDS.contains(&kind));
    let numbered = ["code", "status"]
        .iter()
        .filter_map(|field| problem.get(field).and_then(Value::as_u64))
        .any(|status| REQUEST_STATUSES.contains(&status));
    match named || numbered {
        true => Failure::Request,
        false => Failure::Status,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn only_a_request_the_provider_cannot_take_is_blamed_on_the_request() {
        for status in [400, 413, 422] {
            assert_eq!(of_status(status), Failure::Request, "{status}");
        }
        for status in [401, 403, 404, 408, 429, 500, 502, 503, 529] {
            assert_eq!(of_status(status), Failure::Status, "{status}");
        }
    }

    #[test]
    fn an_error_object_names_its_cause_by_type_code_or_status() {
        for problem in [
            json!({"type": "invalid_request_error", "message": "prompt is too long"}),
            json!({"type": "request_too_large"}),
            json!({"code": "context_length_exceeded", "type": null}),
            json!({"code": 400, "message": "bad"}),
            json!({"status": 422}),
        ] {
            assert_eq!(of_error(&problem), Failure::Request, "{problem}");
        }
        for problem in [
            json!({"type": "overloaded_error"}),
            json!({"type": "rate_limit_error"}),
            json!({"type": "api_error"}),
            json!({"code": 429}),
            json!({"message": "busy"}),
            json!("down"),
        ] {
            assert_eq!(of_error(&problem), Failure::Status, "{problem}");
        }
    }
}
