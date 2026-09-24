use chrono::NaiveDate;
use serde_json::{Map, Value, json};

const STRINGS: &[&str] = &[
    "title",
    "description",
    "genre",
    "tag_list",
    "sharing",
    "isrc",
    "metadata_artist",
    "release_date",
    "permalink",
    "embeddable_by",
    "purchase_url",
    "label_name",
    "release",
    "license",
];
const BOOLEANS: &[&str] = &[
    "streamable",
    "downloadable",
    "commentable",
    "reveal_stats",
    "reveal_comments",
];
const EXTRA_STRINGS: &[&str] = &[
    "permalink",
    "embeddable_by",
    "purchase_url",
    "label_name",
    "release",
    "license",
];

pub struct TrackUpdate {
    body: Value,
    desired: Value,
}

impl TrackUpdate {
    pub fn parse(body: &Value) -> Result<Self, &'static str> {
        let envelope = body
            .as_object()
            .filter(|body| body.len() == 1)
            .ok_or("expected a track object")?;
        let track = envelope
            .get("track")
            .and_then(Value::as_object)
            .filter(|track| !track.is_empty())
            .ok_or("track update is empty")?;
        if body.to_string().len() > 65536 {
            return Err("track update exceeds 64 KiB");
        }
        for (key, value) in track {
            if BOOLEANS.contains(&key.as_str()) {
                if !value.is_boolean() {
                    return Err("track flag must be a boolean");
                }
            } else if STRINGS.contains(&key.as_str()) {
                let value = value.as_str().ok_or("track metadata must be a string")?;
                validate_string(key, value)?;
            } else {
                return Err("unsupported track update field");
            }
        }
        let mut desired = Map::new();
        for key in [
            "title",
            "description",
            "genre",
            "sharing",
            "isrc",
            "metadata_artist",
        ] {
            if let Some(value) = track.get(key).and_then(Value::as_str) {
                desired.insert(key.into(), normalized_string(value));
            }
        }
        if let Some(title) = track.get("title").and_then(Value::as_str) {
            let title = catalog_normalize::unescape_json_unicode(title);
            desired.insert(
                "title_normalized".into(),
                json!(catalog_normalize::normalize_title(&title)),
            );
            desired.insert("title".into(), json!(title));
        }
        if let Some(tags) = track.get("tag_list").and_then(Value::as_str) {
            desired.insert(
                "tags".into(),
                json!(tags.split_whitespace().collect::<Vec<_>>()),
            );
        }
        if let Some(date) = track.get("release_date").and_then(Value::as_str) {
            if date.is_empty() {
                desired.insert("release_date".into(), Value::Null);
                desired.insert("release_year".into(), Value::Null);
            } else {
                let date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
                    .map_err(|_| "invalid release date")?;
                desired.insert("release_date".into(), json!(date.to_string()));
                desired.insert(
                    "release_year".into(),
                    json!(
                        date.format("%Y")
                            .to_string()
                            .parse::<i16>()
                            .map_err(|_| "invalid release year")?
                    ),
                );
            }
        }
        let metadata = metadata_from_sc(&Value::Object(track.clone()));
        let metadata: Map<String, Value> = metadata
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, _)| track.contains_key(*key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if !metadata.is_empty() {
            desired.insert("sc_metadata".into(), Value::Object(metadata));
        }
        Ok(Self {
            body: body.clone(),
            desired: Value::Object(desired),
        })
    }

    pub fn body(&self) -> &Value {
        &self.body
    }

    pub fn desired(&self) -> &Value {
        &self.desired
    }

    pub fn is_private(&self) -> bool {
        self.body.pointer("/track/sharing").and_then(Value::as_str) == Some("private")
    }
}

pub(crate) fn metadata_from_sc(value: &Value) -> Value {
    let mut metadata = Map::new();
    for key in EXTRA_STRINGS {
        let value = value.get(*key).and_then(Value::as_str).or_else(|| {
            (*key == "permalink")
                .then(|| {
                    value
                        .get("permalink_url")
                        .and_then(Value::as_str)
                        .and_then(|url| url.trim_end_matches('/').rsplit('/').next())
                })
                .flatten()
        });
        metadata.insert(
            (*key).into(),
            value.map(normalized_string).unwrap_or(Value::Null),
        );
    }
    for key in BOOLEANS {
        metadata.insert(
            (*key).into(),
            value
                .get(*key)
                .and_then(Value::as_bool)
                .map_or(Value::Null, Value::Bool),
        );
    }
    Value::Object(metadata)
}

fn normalized_string(value: &str) -> Value {
    if value.trim().is_empty() {
        Value::Null
    } else {
        json!(value.trim())
    }
}

fn validate_string(key: &str, value: &str) -> Result<(), &'static str> {
    let valid = match key {
        "title" => !value.trim().is_empty(),
        "sharing" => matches!(value, "public" | "private"),
        "embeddable_by" => matches!(value, "all" | "me" | "none"),
        "license" => matches!(
            value,
            "no-rights-reserved"
                | "all-rights-reserved"
                | "cc-by"
                | "cc-by-nc"
                | "cc-by-nd"
                | "cc-by-sa"
                | "cc-by-nc-nd"
                | "cc-by-nc-sa"
        ),
        "permalink" => {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        }
        "release_date" => {
            value.is_empty()
                || (value.len() == 10 && NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok())
        }
        _ => true,
    };
    if valid {
        Ok(())
    } else {
        Err("invalid track metadata value")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_updates_reject_unknown_fields_and_incorrect_types() {
        for body in [
            json!({}),
            json!({"track": {}}),
            json!({"track": {"duration": 5}}),
            json!({"track": {"sharing": "friends"}}),
            json!({"track": {"title": " "}}),
            json!({"track": {"downloadable": "false"}}),
            json!({"track": {"release_date": "2026-02-30"}}),
        ] {
            assert!(TrackUpdate::parse(&body).is_err(), "{body}");
        }
    }

    #[test]
    fn track_updates_preserve_remote_fields_and_normalize_local_desired_state() {
        let body = json!({"track": {"title": "New title", "description": "", "sharing": "private", "downloadable": false, "release_date": "2026-09-07", "purchase_url": "https://example.com"}});
        let update = TrackUpdate::parse(&body).unwrap();
        assert_eq!(update.body(), &body);
        assert_eq!(update.desired()["description"], Value::Null);
        assert_eq!(update.desired()["release_year"], 2026);
        assert_eq!(update.desired()["sc_metadata"]["downloadable"], false);
        assert!(update.is_private());
    }
}
