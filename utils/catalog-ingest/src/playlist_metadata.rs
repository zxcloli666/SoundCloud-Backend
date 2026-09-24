use chrono::NaiveDate;
use serde_json::{Map, Value, json};

const COLUMNS: &[&str] = &[
    "title",
    "description",
    "genre",
    "sharing",
    "label_name",
    "permalink_url",
];
const EXTRA: &[&str] = &[
    "ean",
    "license",
    "permalink",
    "purchase_title",
    "purchase_url",
    "release",
];

pub struct PlaylistUpdate {
    body: Value,
    desired: Value,
}

impl PlaylistUpdate {
    pub fn parse(body: &Value) -> Result<Self, &'static str> {
        let envelope = body
            .as_object()
            .filter(|value| value.len() == 1)
            .ok_or("expected a playlist object")?;
        let fields = envelope
            .get("playlist")
            .and_then(Value::as_object)
            .filter(|fields| !fields.is_empty())
            .ok_or("playlist update is empty")?;
        if body.to_string().len() > 65536 {
            return Err("playlist update exceeds 64 KiB");
        }
        let mut desired = Map::new();
        let mut metadata = Map::new();
        for (key, value) in fields {
            if !COLUMNS.contains(&key.as_str())
                && !EXTRA.contains(&key.as_str())
                && !matches!(key.as_str(), "tag_list" | "release_date" | "set_type")
            {
                return Err("unsupported playlist metadata field");
            }
            let text = value.as_str().ok_or("playlist metadata must be a string")?;
            match key.as_str() {
                "title" => {
                    if text.trim().is_empty() {
                        return Err("playlist title is empty");
                    }
                    desired.insert("title".into(), json!(text));
                    desired.insert(
                        "title_normalized".into(),
                        json!(catalog_normalize::normalize_title(text)),
                    );
                }
                "sharing" => {
                    if !matches!(text, "public" | "private") {
                        return Err("invalid playlist sharing");
                    }
                    desired.insert(key.clone(), value.clone());
                }
                "set_type" => {
                    if !matches!(text, "album" | "playlist") {
                        return Err("invalid playlist type");
                    }
                    desired.insert("playlist_type".into(), value.clone());
                }
                "tag_list" => {
                    desired.insert(
                        "tags".into(),
                        json!(text.split_whitespace().collect::<Vec<_>>()),
                    );
                }
                "release_date" => {
                    let date = if text.is_empty() {
                        None
                    } else {
                        if text.len() != 10 {
                            return Err("invalid release date");
                        }
                        Some(
                            NaiveDate::parse_from_str(text, "%Y-%m-%d")
                                .map_err(|_| "invalid release date")?,
                        )
                    };
                    desired.insert("release_date".into(), json!(date));
                    desired.insert(
                        "release_year".into(),
                        json!(
                            date.map(|date| date.format("%Y").to_string().parse::<i16>())
                                .transpose()
                                .map_err(|_| "invalid release year")?
                        ),
                    );
                }
                "permalink" => {
                    if text.is_empty()
                        || !text
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
                    {
                        return Err("invalid playlist permalink");
                    }
                    metadata.insert(key.clone(), value.clone());
                }
                _ => {
                    let value = if text.trim().is_empty() {
                        Value::Null
                    } else {
                        json!(text.trim())
                    };
                    if COLUMNS.contains(&key.as_str()) {
                        desired.insert(key.clone(), value);
                    } else {
                        metadata.insert(key.clone(), value);
                    }
                }
            }
        }
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
}

pub(crate) fn metadata_from_sc(payload: &Value) -> Value {
    let fields = EXTRA
        .iter()
        .map(|key| {
            let text = payload.get(*key).and_then(Value::as_str).or_else(|| {
                (*key == "permalink")
                    .then(|| {
                        payload
                            .get("permalink_url")
                            .and_then(Value::as_str)
                            .and_then(|url| url.trim_end_matches('/').rsplit('/').next())
                    })
                    .flatten()
            });
            let value = text
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map_or(Value::Null, |text| json!(text));
            ((*key).to_owned(), value)
        })
        .collect::<Map<_, _>>();
    Value::Object(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_never_accepts_membership_or_response_only_fields() {
        for body in [
            json!({"playlist": {}}),
            json!({"playlist": {"tracks": [{"urn": "soundcloud:tracks:42"}]}}),
            json!({"playlist": {"track_count": 12}}),
            json!({"playlist": {"title": " "}}),
            json!({"playlist": {"set_type": "single"}}),
            json!({"playlist": {"release_date": "2026-02-30"}}),
            json!({"playlist": {"description": null}}),
            json!({"playlist": {"artwork_data": "bytes"}}),
        ] {
            assert!(PlaylistUpdate::parse(&body).is_err(), "{body}");
        }
    }

    #[test]
    fn metadata_preserves_the_remote_patch_and_clears_local_fields() -> Result<(), &'static str> {
        let body = json!({"playlist": {
            "title": "Updated playlist", "description": "", "release_date": "",
            "set_type": "album", "sharing": "private", "purchase_title": "Buy",
        }});
        let update = PlaylistUpdate::parse(&body)?;
        assert_eq!(update.body(), &body);
        assert_eq!(update.desired().get("description"), Some(&Value::Null));
        assert_eq!(update.desired().get("release_date"), Some(&Value::Null));
        assert_eq!(update.desired().get("release_year"), Some(&Value::Null));
        assert_eq!(update.desired().get("playlist_type"), Some(&json!("album")));
        assert_eq!(
            update.desired().pointer("/sc_metadata/purchase_title"),
            Some(&json!("Buy"))
        );
        Ok(())
    }
}
