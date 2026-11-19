use sc_mobile::{MobileApp, client_authorization};
use serde_json::{Map, Value};
use url::form_urlencoded;

pub const REDACTED: &str = "<redacted>";

const SHORTEST_HIDDEN: usize = 4;

const HIDDEN_KEYS: &[&str] = &[
    "accesstoken",
    "refreshtoken",
    "idtoken",
    "token",
    "codeverifier",
    "authorization",
    "trackauthorization",
    "clientid",
    "clientsecret",
    "appid",
    "udid",
    "deviceid",
    "secrettoken",
    "password",
];

pub struct Scrubber {
    hidden: Vec<String>,
}

impl Scrubber {
    pub fn new(app: &MobileApp) -> Self {
        let mut scrubber = Self { hidden: Vec::new() };
        for value in [
            app.client_id.as_str(),
            app.client_secret.expose(),
            app.app_id.as_str(),
            app.app_version.as_str(),
            app.user_agent.as_str(),
            client_authorization(app).expose(),
        ] {
            scrubber.hide(value);
        }
        scrubber
    }

    pub fn hide(&mut self, value: &str) {
        let value = value.trim();
        if value.len() < SHORTEST_HIDDEN {
            return;
        }
        let encoded: String = form_urlencoded::byte_serialize(value.as_bytes()).collect();
        for form in [value.to_owned(), encoded] {
            if !self.hidden.contains(&form) {
                self.hidden.push(form);
            }
        }
        self.hidden
            .sort_by_key(|hidden| std::cmp::Reverse(hidden.len()));
    }

    pub fn value(&self, value: Value) -> Value {
        match value {
            Value::String(text) => Value::String(self.text(&text)),
            Value::Array(items) => {
                Value::Array(items.into_iter().map(|item| self.value(item)).collect())
            }
            Value::Object(fields) => Value::Object(self.object(fields)),
            plain => plain,
        }
    }

    pub fn text(&self, text: &str) -> String {
        let mut text = text.to_owned();
        for hidden in &self.hidden {
            if text.contains(hidden.as_str()) {
                text = text.replace(hidden.as_str(), REDACTED);
            }
        }
        hide_url_queries(&text)
    }

    fn object(&self, fields: Map<String, Value>) -> Map<String, Value> {
        fields
            .into_iter()
            .map(|(key, value)| {
                let value = if hides(&key) && !value.is_null() {
                    Value::String(REDACTED.to_owned())
                } else {
                    self.value(value)
                };
                (key, value)
            })
            .collect()
    }
}

fn hides(key: &str) -> bool {
    let key: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect();
    HIDDEN_KEYS.contains(&key.as_str()) || key.contains("email") || key.contains("phone")
}

fn hide_url_queries(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("http") {
        let (before, from) = rest.split_at(start);
        out.push_str(before);
        let end = from
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '\\'))
            .unwrap_or(from.len());
        let (candidate, after) = from.split_at(end);
        out.push_str(&hide_query(candidate));
        rest = after;
    }
    out.push_str(rest);
    out
}

fn hide_query(candidate: &str) -> String {
    let is_url = candidate.starts_with("https://") || candidate.starts_with("http://");
    let Some((base, query)) = candidate.split_once('?').filter(|_| is_url) else {
        return candidate.to_owned();
    };
    let query = query.split('#').next().unwrap_or_default();
    let names: Vec<String> = form_urlencoded::parse(query.as_bytes())
        .map(|(name, _)| format!("{name}={REDACTED}"))
        .collect();
    format!("{base}?{}", names.join("&"))
}

#[cfg(test)]
mod tests {
    use redact::Secret;
    use serde_json::json;

    use super::*;

    fn scrubber() -> Scrubber {
        Scrubber::new(&MobileApp {
            client_id: "dummy-client".to_owned(),
            client_secret: Secret::new("dummy-secret".to_owned()),
            app_id: "dummy-app".to_owned(),
            app_version: "dummy-version".to_owned(),
            user_agent: "Dummy Agent/1.0".to_owned(),
        })
    }

    #[test]
    fn known_values_vanish_wherever_they_appear() {
        let mut scrubber = scrubber();
        scrubber.hide("dummy-access-token");

        let scrubbed = scrubber.value(json!({
            "note": "sent dummy-access-token as dummy-client from Dummy+Agent%2F1.0",
            "nested": [{ "access_token": "anything", "refresh_token": null, "code": 404 }],
        }));

        assert_eq!(
            scrubbed,
            json!({
                "note": "sent <redacted> as <redacted> from <redacted>",
                "nested": [{ "access_token": "<redacted>", "refresh_token": null, "code": 404 }],
            })
        );
    }

    #[test]
    fn url_queries_keep_their_names_and_lose_their_values() {
        let scrubbed = scrubber().text(
            "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"https://keys.example/k?Policy=abc&Signature=def\"\nhttps://cdn.example/a.mp3?Key-Pair-Id=xyz#frag\nhttp-ish text",
        );

        assert_eq!(
            scrubbed,
            "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"https://keys.example/k?Policy=<redacted>&Signature=<redacted>\"\nhttps://cdn.example/a.mp3?Key-Pair-Id=<redacted>\nhttp-ish text"
        );
    }

    #[test]
    fn personal_fields_are_hidden_by_name() {
        let scrubbed = scrubber().value(
            json!({ "primary_email": "a@b.c", "secretToken": "s-1", "urn": "soundcloud:users:1" }),
        );

        assert_eq!(
            scrubbed,
            json!({ "primary_email": "<redacted>", "secretToken": "<redacted>", "urn": "soundcloud:users:1" })
        );
    }
}
