use chrono::NaiveDate;
use serde_json::Value;

pub fn extract(track: &Value) -> (Option<i16>, Option<NaiveDate>) {
    if let Some(value) = track.get("release_date") {
        if value.is_null() || value.as_str() == Some("") {
            return (None, None);
        }
        if let Some(date) = value
            .as_str()
            .and_then(|value| value.get(..10))
            .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        {
            let year = date.format("%Y").to_string().parse::<i16>().ok();
            return (year, Some(date));
        }
    }
    let year = track
        .get("release_year")
        .and_then(Value::as_i64)
        .and_then(|value| i16::try_from(value).ok());
    let month = track
        .get("release_month")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    let day = track
        .get("release_day")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    let date = year
        .zip(month)
        .zip(day)
        .and_then(|((year, month), day)| NaiveDate::from_ymd_opt(i32::from(year), month, day));
    (year, date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cleared_release_dates_do_not_become_upload_dates() {
        assert_eq!(
            extract(
                &json!({"created_at": "2026-09-07T12:00:00Z", "display_date": "2026-09-07T12:00:00Z"})
            ),
            (None, None)
        );
        assert_eq!(
            extract(&json!({"release_year": 2026, "release_month": 9, "release_day": 7})),
            (Some(2026), NaiveDate::from_ymd_opt(2026, 9, 7))
        );
    }

    #[test]
    fn malformed_release_dates_do_not_panic_on_unicode() {
        assert_eq!(
            extract(&json!({"release_date": "123456789🎵"})),
            (None, None)
        );
    }
}
