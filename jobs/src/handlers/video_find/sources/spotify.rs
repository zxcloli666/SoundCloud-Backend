use backend_contracts::admin_video::VideoSource;
use backend_contracts::pipeline::{VideoCandidate, VideoCandidateKind};
use bytes::Bytes;
use catalog_match::{DurationMatch, artist_score, duration_match, title_score};
use serde_json::{Value, json};

use super::{Failure, Wanted, failure_of};
use crate::handlers::video_accounts::{BROWSER_AGENT, Fetch, SpotifyKey, access_token, headers};

const SEARCH: &str = "https://api.spotify.com/v1/search";
const PATHFINDER: &str = "https://api-partner.spotify.com/pathfinder/v2/query";
const CANVAS_HASH: &str = "575138ab27cd5c1b3e54da54d0a7cc8d85485402de26340c2145f0f6bb5e7a9f";
const TITLE_MIN: f32 = 0.95;
const ARTIST_MIN: f32 = 0.85;

pub(super) async fn canvas<F: Fetch>(
    fetch: &F,
    key: &SpotifyKey,
    sp_dc: &str,
    wanted: &Wanted,
) -> Result<Option<VideoCandidate>, Failure> {
    let token = access_token(fetch, key, sp_dc).await?;
    let Some(uri) = find_track(fetch, &token, wanted).await? else {
        return Ok(None);
    };
    let reply = query_canvas(fetch, &token, &uri).await?;
    Ok(canvas_url(&reply).map(|url| VideoCandidate {
        source: VideoSource::Spotify,
        kind: VideoCandidateKind::Url,
        value: url,
        trusted: true,
        source_ref: Some(uri),
    }))
}

async fn find_track<F: Fetch>(
    fetch: &F,
    token: &str,
    wanted: &Wanted,
) -> Result<Option<String>, Failure> {
    if let Some(isrc) = wanted.isrc.as_deref().filter(|isrc| !isrc.is_empty()) {
        let reply = search(fetch, token, &format!("isrc:{isrc}")).await?;
        if let Some(uri) = tracks(&reply).first().map(|found| found.uri.clone()) {
            return Ok(Some(uri));
        }
    }
    let query = format!("track:{} artist:{}", wanted.title, wanted.artist);
    let reply = search(fetch, token, &query).await?;
    Ok(tracks(&reply)
        .into_iter()
        .find(|found| same_track(found, wanted))
        .map(|found| found.uri))
}

async fn search<F: Fetch>(fetch: &F, token: &str, query: &str) -> Result<Value, Failure> {
    let url = format!(
        "{SEARCH}?type=track&limit=5&q={}",
        urlencoding::encode(query)
    );
    let bearer = format!("Bearer {token}");
    let request = headers(&[("authorization", &bearer), ("accept", "application/json")])?;
    let body = fetch
        .get(&url, request)
        .await
        .map_err(|error| failure_of("Spotify", &error))?;
    parse(&body)
}

async fn query_canvas<F: Fetch>(fetch: &F, token: &str, uri: &str) -> Result<Value, Failure> {
    let bearer = format!("Bearer {token}");
    let request = headers(&[
        ("authorization", &bearer),
        ("content-type", "application/json"),
        ("accept", "application/json"),
        ("user-agent", BROWSER_AGENT),
        ("app-platform", "WebPlayer"),
    ])?;
    let body = json!({
        "operationName": "canvas",
        "variables": {"trackUri": uri},
        "extensions": {"persistedQuery": {"version": 1, "sha256Hash": CANVAS_HASH}},
    });
    let reply = fetch
        .post(PATHFINDER, request, Bytes::from(body.to_string()))
        .await
        .map_err(|error| failure_of("Spotify", &error))?;
    parse(&reply)
}

fn parse(body: &Bytes) -> Result<Value, Failure> {
    serde_json::from_slice(body)
        .map_err(|_| Failure::Unavailable("Spotify answered with an unexpected body".to_owned()))
}

struct Found {
    uri: String,
    title: String,
    artists: Vec<String>,
    duration_ms: Option<i64>,
}

fn tracks(reply: &Value) -> Vec<Found> {
    let items = reply["tracks"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    items
        .iter()
        .filter_map(|item| {
            Some(Found {
                uri: item["uri"].as_str()?.to_owned(),
                title: item["name"].as_str()?.to_owned(),
                artists: item["artists"]
                    .as_array()?
                    .iter()
                    .filter_map(|artist| artist["name"].as_str().map(str::to_owned))
                    .collect(),
                duration_ms: item["duration_ms"].as_i64(),
            })
        })
        .collect()
}

fn same_track(found: &Found, wanted: &Wanted) -> bool {
    let artist = found
        .artists
        .iter()
        .map(|name| artist_score(&wanted.artist, Some(name), None))
        .fold(0.0_f32, f32::max);
    title_score(&wanted.title, &found.title, None) >= TITLE_MIN
        && artist >= ARTIST_MIN
        && matches!(
            duration_match(wanted.duration_ms, found.duration_ms),
            DurationMatch::Exact | DurationMatch::Close
        )
}

fn canvas_url(reply: &Value) -> Option<String> {
    match reply {
        Value::String(text) if text.starts_with("https://") && text.contains(".mp4") => {
            Some(text.clone())
        }
        Value::Array(items) => items.iter().find_map(canvas_url),
        Value::Object(fields) => fields.values().find_map(canvas_url),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_canvas_url_is_the_first_video_link_in_the_reply() {
        let reply = json!({"data": {"trackUnion": {"canvas": {
            "type": "VIDEO",
            "thumbnail": "https://i.scdn.co/image/x",
            "url": "https://canvaz.scdn.co/upload/a.cnvs.mp4"
        }}}});

        assert_eq!(
            canvas_url(&reply).as_deref(),
            Some("https://canvaz.scdn.co/upload/a.cnvs.mp4")
        );
    }

    #[test]
    fn an_image_canvas_has_no_video_url() {
        let reply = json!({"data": {"trackUnion": {"canvas": {"type": "IMAGE", "url": "https://i.scdn.co/image/y"}}}});

        assert_eq!(canvas_url(&reply), None);
    }
}
