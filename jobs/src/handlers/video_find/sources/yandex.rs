use backend_contracts::admin_video::VideoSource;
use backend_contracts::pipeline::{VideoCandidate, VideoCandidateKind};
use catalog_match::{DurationMatch, artist_score, duration_match, title_score};
use serde_json::Value;

use super::{Failure, Wanted, failure_of};
use crate::handlers::video_accounts::{Fetch, headers};

const API: &str = "https://api.music.yandex.net";
const RUSSIAN_EXIT: &str = "geo-ru";
const TITLE_MIN: f32 = 0.95;
const ARTIST_MIN: f32 = 0.85;
const MAX_CLIPS: usize = 2;

pub(super) async fn clips<F: Fetch>(
    fetch: &F,
    token: &str,
    wanted: &Wanted,
) -> Result<Vec<VideoCandidate>, Failure> {
    let query = format!("{} {}", wanted.artist, wanted.title);
    let url = format!(
        "{API}/search?type=track&page=0&text={}",
        urlencoding::encode(&query)
    );
    let found = get(fetch, token, &url).await?;
    let Some(track) = pick(&found, wanted) else {
        return Ok(Vec::new());
    };
    let supplement = get(fetch, token, &format!("{API}/tracks/{track}/supplement")).await?;
    Ok(videos(&supplement, &track))
}

async fn get<F: Fetch>(fetch: &F, token: &str, url: &str) -> Result<Value, Failure> {
    let authorization = format!("OAuth {token}");
    let request = headers(&[
        ("authorization", &authorization),
        ("x-egress-tag", RUSSIAN_EXIT),
        ("accept", "application/json"),
        ("user-agent", "Yandex-Music-API"),
    ])?;
    let body = fetch
        .get(url, request)
        .await
        .map_err(|error| failure_of("Yandex Music", &error))?;
    serde_json::from_slice(&body)
        .map_err(|_| Failure::Unavailable("Yandex answered with an unexpected body".to_owned()))
}

fn pick(reply: &Value, wanted: &Wanted) -> Option<String> {
    let results = reply["result"]["tracks"]["results"].as_array()?;
    results.iter().find_map(|item| {
        let id = item["id"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| item["id"].as_i64().map(|id| id.to_string()))?;
        let title = item["title"].as_str()?;
        let artist = item["artists"]
            .as_array()?
            .iter()
            .filter_map(|artist| artist["name"].as_str())
            .map(|name| artist_score(&wanted.artist, Some(name), None))
            .fold(0.0_f32, f32::max);
        let close = matches!(
            duration_match(wanted.duration_ms, item["durationMs"].as_i64()),
            DurationMatch::Exact | DurationMatch::Close
        );
        (title_score(&wanted.title, title, None) >= TITLE_MIN && artist >= ARTIST_MIN && close)
            .then_some(id)
    })
}

fn videos(reply: &Value, track: &str) -> Vec<VideoCandidate> {
    let Some(items) = reply["result"]["videos"].as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            item["url"]
                .as_str()
                .or_else(|| item["embedUrl"].as_str())
                .filter(|url| url.starts_with("https://"))
        })
        .take(MAX_CLIPS)
        .map(|url| VideoCandidate {
            source: VideoSource::Yandex,
            kind: VideoCandidateKind::Url,
            value: url.to_owned(),
            trusted: false,
            source_ref: Some(track.to_owned()),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn supplement_videos_become_untrusted_candidates_of_the_track() {
        let reply = json!({"result": {"videos": [
            {"url": "https://example.test/v/1"},
            {"embedUrl": "https://example.test/e/2"},
            {"url": "http://plain.test/3"}
        ]}});

        let found = videos(&reply, "77");

        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|clip| !clip.trusted));
        assert_eq!(found[1].value, "https://example.test/e/2");
        assert_eq!(found[0].source_ref.as_deref(), Some("77"));
    }
}
