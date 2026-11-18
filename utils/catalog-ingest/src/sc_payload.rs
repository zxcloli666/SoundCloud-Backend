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

pub fn permalink_field(payload: &Value, key: &str) -> Option<String> {
    string_field(payload, key).map(|url| clean_permalink(&url))
}

pub fn clean_permalink(url: &str) -> String {
    let Some((base, query)) = url.split_once('?') else {
        return url.to_owned();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| !pair.is_empty() && !pair.starts_with("utm_"))
        .collect();
    if kept.is_empty() {
        base.to_owned()
    } else {
        format!("{base}?{}", kept.join("&"))
    }
}

pub fn permalink_slug(url: &str) -> Option<&str> {
    url.split('?')
        .next()?
        .trim_end_matches('/')
        .rsplit('/')
        .next()
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

    #[test]
    fn a_share_tail_leaves_the_permalink_and_other_params_stay() {
        let base = "https://soundcloud.com/artist/song";
        let shared =
            format!("{base}?utm_medium=api&utm_campaign=social_sharing&utm_source=id_339169");
        assert_eq!(clean_permalink(&shared), base);
        assert_eq!(clean_permalink(base), base);
        assert_eq!(
            clean_permalink("https://soundcloud.com/a/song/s-Ab12Cd?utm_medium=api"),
            "https://soundcloud.com/a/song/s-Ab12Cd"
        );
        assert_eq!(
            clean_permalink("https://soundcloud.com/a/song?secret_token=s-Ab&utm_source=x"),
            "https://soundcloud.com/a/song?secret_token=s-Ab"
        );
        assert_eq!(
            permalink_slug("https://soundcloud.com/artist/song/?utm_medium=api"),
            Some("song")
        );
        assert_eq!(permalink_slug(&shared), Some("song"));
    }

    #[test]
    fn tracks_users_and_playlists_keep_one_permalink_for_both_forms() {
        let clean = "https://soundcloud.com/artist/song";
        let shared =
            format!("{clean}?utm_medium=api&utm_campaign=social_sharing&utm_source=id_339169");
        for url in [clean.to_owned(), shared] {
            let track = crate::ScTrackFields::from_sc(
                &json!({"id": 1, "title": "Song", "permalink_url": url, "user": {"id": 2}}),
            )
            .unwrap();
            assert_eq!(track.permalink_url.as_deref(), Some(clean));
            assert_eq!(track.sc_metadata["permalink"], "song");
            let user = crate::user::ScUserFields::from_sc(
                &json!({"id": 2, "username": "artist", "permalink_url": url}),
            )
            .unwrap();
            assert_eq!(user.permalink_url.as_deref(), Some(clean));
            let playlist = crate::playlist::ScPlaylistFields::from_sc(
                &json!({"id": 3, "title": "Mix", "permalink_url": url}),
            )
            .unwrap();
            assert_eq!(playlist.permalink_url.as_deref(), Some(clean));
        }
    }
}
