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
