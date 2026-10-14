use chrono::{DateTime, Utc};
use serde_json::Value;

pub fn string_field(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

pub fn without_nul(payload: &Value) -> Value {
    match payload {
        Value::String(text) => Value::String(text.replace('\0', "")),
        Value::Array(items) => Value::Array(items.iter().map(without_nul).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| (key.replace('\0', ""), without_nul(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

pub fn parse_dt(value: Option<&Value>) -> Option<DateTime<Utc>> {
    let s = value.and_then(|v| v.as_str())?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y/%m/%d %H:%M:%S %z")
                .ok()
                .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn without_nul_cleans_nested_strings_and_keys() -> serde_json::Result<()> {
        let payload: Value = serde_json::from_str(
            r#"{"user": {"username": "A\u0000B", "tags\u0000": ["x\u0000"]}, "count": 3}"#,
        )?;
        assert_eq!(
            without_nul(&payload),
            json!({"user": {"username": "AB", "tags": ["x"]}, "count": 3})
        );
        Ok(())
    }
}
