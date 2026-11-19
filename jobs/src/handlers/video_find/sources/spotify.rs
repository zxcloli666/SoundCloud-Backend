use backend_contracts::admin_video::VideoSource;
use backend_contracts::pipeline::VideoCandidateKind;
use bytes::Bytes;
use catalog_match::{DurationMatch, artist_score, duration_match, title_score};
use serde_json::{Value, json};

use super::token::Tokens;
use super::{Failure, Lead, Wanted, failure_of};
use crate::handlers::video_accounts::{BROWSER_AGENT, Fetch, SpotifyKey, headers};

const PATHFINDER: &str = "https://api-partner.spotify.com/pathfinder/v2/query";
const SEARCH_HASH: &str = "b02683192a98dde7966b5e6655a79eeb62713eab703eda9902c932818dd52751";
const CANVAS_HASH: &str = "575138ab27cd5c1b3e54da54d0a7cc8d85485402de26340c2145f0f6bb5e7a9f";
const SEARCH_LIMIT: u32 = 5;
const TITLE_MIN: f32 = 0.95;
const ARTIST_MIN: f32 = 0.85;

pub(super) async fn canvas<F: Fetch>(
    fetch: &F,
    tokens: &Tokens,
    key: &SpotifyKey,
    sp_dc: &str,
    wanted: &Wanted,
) -> Result<Option<Lead>, Failure> {
    loop {
        let (token, reused) = tokens.get(fetch, key, sp_dc).await?;
        let answer = lookup(fetch, &token, wanted).await;
        if matches!(answer, Err(Failure::Rejected(_))) {
            tokens.forget(sp_dc).await;
            if reused {
                continue;
            }
        }
        return answer;
    }
}

async fn lookup<F: Fetch>(
    fetch: &F,
    token: &str,
    wanted: &Wanted,
) -> Result<Option<Lead>, Failure> {
    let Some(uri) = find_track(fetch, token, wanted).await? else {
        return Ok(None);
    };
    let reply = query_canvas(fetch, token, &uri).await?;
    Ok(canvas_url(&reply).map(|url| Lead {
        source: VideoSource::Spotify,
        kind: VideoCandidateKind::Canvas,
        url,
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
        if let Some(found) = tracks(&reply).into_iter().next() {
            return Ok(Some(found.uri));
        }
    }
    let query = format!("{} {}", wanted.title, wanted.artist);
    let reply = search(fetch, token, &query).await?;
    Ok(tracks(&reply)
        .into_iter()
        .find(|found| same_track(found, wanted))
        .map(|found| found.uri))
}

async fn search<F: Fetch>(fetch: &F, token: &str, query: &str) -> Result<Value, Failure> {
    let variables = json!({
        "searchTerm": query,
        "offset": 0,
        "limit": SEARCH_LIMIT,
        "numberOfTopResults": SEARCH_LIMIT,
        "includeAudiobooks": false,
        "includePreReleases": true,
        "includeAuthors": false,
    });
    pathfinder(
        fetch,
        token,
        "Spotify search",
        "searchTracks",
        SEARCH_HASH,
        variables,
    )
    .await
}

async fn query_canvas<F: Fetch>(fetch: &F, token: &str, uri: &str) -> Result<Value, Failure> {
    let variables = json!({"trackUri": uri});
    pathfinder(
        fetch,
        token,
        "Spotify canvas",
        "canvas",
        CANVAS_HASH,
        variables,
    )
    .await
}

async fn pathfinder<F: Fetch>(
    fetch: &F,
    token: &str,
    step: &str,
    operation: &str,
    hash: &str,
    variables: Value,
) -> Result<Value, Failure> {
    let bearer = format!("Bearer {token}");
    let request = headers(&[
        ("authorization", &bearer),
        ("content-type", "application/json"),
        ("accept", "application/json"),
        ("user-agent", BROWSER_AGENT),
        ("app-platform", "WebPlayer"),
    ])?;
    let body = json!({
        "operationName": operation,
        "variables": variables,
        "extensions": {"persistedQuery": {"version": 1, "sha256Hash": hash}},
    });
    let reply = fetch
        .post(PATHFINDER, request, Bytes::from(body.to_string()))
        .await
        .map_err(|error| failure_of(step, &error))?;
    parse(step, &reply)
}

fn parse(step: &str, body: &Bytes) -> Result<Value, Failure> {
    serde_json::from_slice(body)
        .map_err(|_| Failure::Unavailable(format!("{step} answered with an unexpected body")))
}

struct Found {
    uri: String,
    title: String,
    artists: Vec<String>,
    duration_ms: Option<i64>,
}

fn tracks(reply: &Value) -> Vec<Found> {
    let items = reply["data"]["searchV2"]["tracksV2"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    items
        .iter()
        .filter_map(|item| {
            let track = &item["item"]["data"];
            Some(Found {
                uri: track["uri"].as_str()?.to_owned(),
                title: track["name"].as_str()?.to_owned(),
                artists: track["artists"]["items"]
                    .as_array()?
                    .iter()
                    .filter_map(|artist| artist["profile"]["name"].as_str().map(str::to_owned))
                    .collect(),
                duration_ms: track["duration"]["totalMilliseconds"].as_i64(),
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

    const FOUND: &str = include_str!("fixtures/search_tracks.json");

    #[test]
    fn a_pathfinder_search_reply_gives_uri_title_artists_and_duration() {
        let reply: Value = serde_json::from_str(FOUND).unwrap();

        let found = tracks(&reply);

        assert_eq!(found.len(), 5);
        assert_eq!(found[0].uri, "spotify:track:5zSu3mGfeaY3q3xDXFVBa0");
        assert_eq!(found[0].title, "gothrussia");
        assert_eq!(found[0].artists, ["Psychosis"]);
        assert_eq!(found[0].duration_ms, Some(171_756));
    }

    #[test]
    fn a_reply_without_tracks_is_empty() {
        assert!(tracks(&json!({"data": {"searchV2": {}}})).is_empty());
    }

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
