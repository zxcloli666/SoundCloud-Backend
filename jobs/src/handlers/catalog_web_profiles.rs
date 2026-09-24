use serde_json::Value;

use crate::queue::{JobError, JobResult};

pub(super) fn validate(value: &Value) -> JobResult {
    let invalid = || JobError::retryable(anyhow::anyhow!("invalid or incomplete web profiles"));
    let items = value.as_array().ok_or_else(invalid)?;
    if items.len() >= 200
        || serde_json::to_vec(value)
            .map_err(JobError::retryable)?
            .len()
            > 131072
    {
        return Err(invalid());
    }
    for item in items {
        let object = item.as_object().ok_or_else(invalid)?;
        let url = object
            .get("url")
            .and_then(Value::as_str)
            .and_then(|url| wreq::Url::parse(url).ok())
            .ok_or_else(invalid)?;
        if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
            return Err(invalid());
        }
        for field in ["urn", "kind", "service", "title", "username", "created_at"] {
            if object
                .get(field)
                .is_some_and(|value| !value.is_null() && !value.is_string())
            {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn web_profiles_accept_empty_and_link_snapshots() {
        assert!(validate(&json!([])).is_ok());
        assert!(validate(&json!([{"url":"https://example.test", "title":null}])).is_ok());
    }

    #[test]
    fn web_profiles_reject_error_envelopes_bad_links_and_truncated_lists() {
        for value in [
            Value::Null,
            json!({"errors": []}),
            json!({"collection": [], "next_href": "https://example.test/next"}),
            json!([{}]),
            json!([{"url":"javascript:alert(1)"}]),
            json!([{"url":"https://example.test", "title":false}]),
            json!(vec![json!({"url":"https://example.test"}); 200]),
            json!([{"url":"https://example.test", "title":"x".repeat(131072)}]),
        ] {
            assert!(validate(&value).is_err());
        }
    }
}
