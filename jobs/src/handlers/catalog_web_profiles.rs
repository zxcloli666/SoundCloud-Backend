use serde_json::Value;

use crate::queue::{JobError, JobResult};

pub(super) fn links_only(mut value: Value) -> Value {
    if let Some(items) = value.as_array_mut() {
        items.retain(|item| {
            item.get("url")
                .and_then(Value::as_str)
                .is_some_and(is_web_link)
        });
    }
    value
}

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
            .ok_or_else(invalid)?;
        if !is_web_link(url) {
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

fn is_web_link(url: &str) -> bool {
    url::Url::parse(url)
        .is_ok_and(|url| matches!(url.scheme(), "https" | "http") && url.host_str().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn apiv2_web_profiles_pass_as_apiv1_links_with_a_service() {
        let mut links = json!([
            {"url": "https://www.instagram.com/bigsean/", "network": "instagram", "title": "", "username": "bigsean"},
            {"url": "http://uknowbigsean.com/", "network": "personal", "title": "", "username": ""}
        ]);
        sc_transport::normalize_v2_to_v1(&mut links);
        assert!(validate(&links).is_ok());
        assert_eq!(links[0]["service"], "instagram");
        assert_eq!(links[1]["service"], "personal");
    }

    #[test]
    fn apiv2_emails_and_script_links_are_dropped_and_web_links_kept() {
        let links = links_only(json!([
            {"url": "https://open.spotify.com/artist/0P84", "network": "spotify", "title": "SPOTIFY"},
            {"url": "sweatbuh@gmail.com", "network": "other", "title": "beat/song enquiry"},
            {"url": "javascript:alert(1)", "network": "other"},
            {"url": "http://t.me/x", "network": "personal", "title": "tg"}
        ]));
        assert!(validate(&links).is_ok());
        let urls: Vec<&str> = links
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|link| link["url"].as_str())
            .collect();
        assert_eq!(
            urls,
            ["https://open.spotify.com/artist/0P84", "http://t.me/x"]
        );
    }

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
