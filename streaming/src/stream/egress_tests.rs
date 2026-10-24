use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;

use super::anon::AnonClient;
use super::egress::{TAG_HEADER, parse_regions};
use super::quality;

const TRACK_ID: &str = "1696778178";
const TRACK_URN: &str = "soundcloud:tracks:1696778178";
const PLAYLIST: &str = "https://playback.media-streaming.soundcloud.cloud/cenc/x/playlist.m3u8";
const MP3: &str = "https://cf-media.sndcdn.com/x.128.mp3";

type Asked = Vec<(String, Option<String>)>;

#[derive(Clone, Default)]
struct World {
    open_in: Arc<Mutex<Vec<Option<String>>>>,
    resolves_in: Arc<Mutex<Option<Vec<Option<String>>>>>,
    plain: Arc<AtomicBool>,
    sq_only: Arc<AtomicBool>,
    absent: Arc<Mutex<Vec<Option<String>>>>,
    asked: Arc<Mutex<Asked>>,
}

impl World {
    fn open_in(&self, regions: &[Option<&str>]) {
        *self.open_in.lock().unwrap() = tags(regions);
    }

    fn resolves_in(&self, regions: &[Option<&str>]) {
        *self.resolves_in.lock().unwrap() = Some(tags(regions));
    }

    fn sees(&self, tag: &Option<String>) -> bool {
        self.open_in.lock().unwrap().contains(tag)
    }

    fn resolves(&self, tag: &Option<String>) -> bool {
        match &*self.resolves_in.lock().unwrap() {
            Some(regions) => regions.contains(tag),
            None => self.sees(tag),
        }
    }

    fn take_asked(&self, host_and_path: &str) -> Vec<Option<String>> {
        let mut asked = self.asked.lock().unwrap();
        let (matching, rest): (Vec<_>, Vec<_>) = asked
            .drain(..)
            .partition(|(target, _)| target.contains(host_and_path));
        *asked = rest;
        matching.into_iter().map(|(_, tag)| tag).collect()
    }
}

async fn intermediate_proxy(State(world): State<World>, headers: HeaderMap) -> Response {
    let target = headers
        .get("x-target")
        .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
        .and_then(|v| String::from_utf8(v).ok())
        .unwrap_or_default();
    let tag = headers
        .get(TAG_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    world
        .asked
        .lock()
        .unwrap()
        .push((target.clone(), tag.clone()));

    if world.absent.lock().unwrap().contains(&tag) {
        return (StatusCode::BAD_GATEWAY, "no proxy carries this tag").into_response();
    }
    let Ok(url) = url::Url::parse(&target) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    match (url.host_str().unwrap_or(""), url.path()) {
        ("soundcloud.com", "/") => {
            r#"<script>window.__sc_hydration = [{"hydratable":"apiClient","data":{"id":"CID","isExpiring":false}}];</script>"#
                .into_response()
        }
        ("api-v2.soundcloud.com", path) if path.starts_with("/tracks/") => {
            if world.sq_only.load(Ordering::SeqCst) && world.sees(&tag) {
                sq_only_track().into_response()
            } else if world.sees(&tag) {
                open_track(world.plain.load(Ordering::SeqCst)).into_response()
            } else {
                r#"{"policy":"BLOCK","track_authorization":"auth","media":{"transcodings":[]}}"#
                    .into_response()
            }
        }
        ("api-v2.soundcloud.com", path) if path.starts_with("/media/") => {
            if !world.resolves(&tag) || path.contains("/abr_sq/") {
                (StatusCode::NOT_FOUND, "{}").into_response()
            } else if path.ends_with("/progressive") {
                format!(r#"{{"url":"{MP3}"}}"#).into_response()
            } else {
                format!(r#"{{"url":"{PLAYLIST}","licenseAuthToken":"license"}}"#).into_response()
            }
        }
        ("playback.media-streaming.soundcloud.cloud", _) => "#EXTM3U\n".into_response(),
        ("cf-media.sndcdn.com", _) => b"ID3\x04\x00mp3 frames".to_vec().into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

fn open_track(plain: bool) -> String {
    let transcoding = if plain {
        serde_json::json!({
            "url": format!("https://api-v2.soundcloud.com/media/{TRACK_URN}/x/stream/progressive"),
            "preset": "mp3_1_0",
            "snipped": false,
            "quality": "sq",
            "format": { "protocol": "progressive", "mime_type": "audio/mpeg" }
        })
    } else {
        serde_json::json!({
            "url": format!("https://api-v2.soundcloud.com/media/{TRACK_URN}/x/stream/ctr-encrypted-hls"),
            "preset": "aac_160k",
            "snipped": false,
            "quality": "sq",
            "format": { "protocol": "ctr-encrypted-hls", "mime_type": "audio/mp4; codecs=\"mp4a.40.2\"" }
        })
    };
    serde_json::json!({
        "policy": "MONETIZE",
        "track_authorization": "auth",
        "media": { "transcodings": [transcoding] }
    })
    .to_string()
}

fn sq_only_track() -> String {
    let encrypted = |preset: &str, quality: &str| {
        serde_json::json!({
            "url": format!("https://api-v2.soundcloud.com/media/{TRACK_URN}/{preset}/stream/ctr-encrypted-hls"),
            "preset": preset,
            "snipped": false,
            "quality": quality,
            "format": { "protocol": "ctr-encrypted-hls", "mime_type": "audio/mp4; codecs=\"mp4a.40.2\"" }
        })
    };
    serde_json::json!({
        "policy": "MONETIZE",
        "track_authorization": "auth",
        "media": { "transcodings": [encrypted("aac_96k", "lq"), encrypted("abr_sq", "sq")] }
    })
    .to_string()
}

async fn anon_behind(world: &World) -> Arc<AnonClient> {
    let app = axum::Router::new()
        .fallback(intermediate_proxy)
        .with_state(world.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port is free");
    let address = listener.local_addr().expect("the listener has an address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let anon = AnonClient::new(wreq::Client::new(), format!("http://{address}/"))
        .with_regions(parse_regions("geo-us,geo-fr"));
    anon.get_client_id()
        .await
        .expect("the page names a client id");
    world.take_asked("soundcloud.com/");
    Arc::new(anon)
}

fn tags(tags: &[Option<&str>]) -> Vec<Option<String>> {
    tags.iter().map(|t| t.map(str::to_owned)).collect()
}

#[tokio::test]
async fn a_track_blocked_where_we_ask_first_is_read_and_resolved_through_a_region_that_sees_it() {
    let world = World::default();
    world.open_in(&[Some("geo-fr")]);
    let anon = anon_behind(&world).await;

    let src = anon
        .resolve_restricted(TRACK_URN, false, quality::SQ)
        .await
        .expect("the region that sees the track answers")
        .expect("the track has a full encrypted transcoding");

    assert_eq!(src.token, "license");
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/tracks/"),
        tags(&[None, Some("geo-us"), Some("geo-fr")])
    );
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/media/"),
        tags(&[Some("geo-fr")]),
        "soundcloud checks the region again when the transcoding is resolved; a resolve from \
         another country answers 404 even with a valid track_authorization"
    );
    assert_eq!(
        world.take_asked("playback.media-streaming.soundcloud.cloud"),
        tags(&[None]),
        "the manifest host is not region checked and must never see our routing header"
    );
}

#[tokio::test]
async fn the_region_that_answered_is_asked_first_next_time() {
    let world = World::default();
    world.open_in(&[Some("geo-fr")]);
    let anon = anon_behind(&world).await;
    anon.read_track(TRACK_ID).await.expect("the track is read");
    world.take_asked("api-v2.soundcloud.com/tracks/");

    let read = anon.read_track(TRACK_ID).await.expect("the track is read");

    assert_eq!(read.region.as_deref(), Some("geo-fr"));
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/tracks/"),
        tags(&[Some("geo-fr")]),
        "every play after the first goes straight to the region that sees the track"
    );
}

#[tokio::test]
async fn a_remembered_region_that_closed_is_dropped_for_one_that_sees_the_track() {
    let world = World::default();
    world.open_in(&[Some("geo-fr")]);
    let anon = anon_behind(&world).await;
    anon.read_track(TRACK_ID).await.expect("the track is read");
    world.take_asked("api-v2.soundcloud.com/tracks/");

    world.open_in(&[Some("geo-us")]);
    let read = anon.read_track(TRACK_ID).await.expect("the track is read");

    assert_eq!(read.region.as_deref(), Some("geo-us"));
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/tracks/"),
        tags(&[Some("geo-fr"), None, Some("geo-us")])
    );
}

#[tokio::test]
async fn a_track_blocked_everywhere_costs_one_read_for_the_next_hour() {
    let world = World::default();
    let anon = anon_behind(&world).await;

    let first = anon.read_track(TRACK_ID).await.expect("soundcloud answers");
    assert!(first.region.is_none());
    assert_eq!(world.take_asked("api-v2.soundcloud.com/tracks/").len(), 3);

    let error = anon
        .get_stream(TRACK_URN, quality::SQ)
        .await
        .err()
        .expect("nothing is open anywhere");
    assert!(
        error.to_string().contains("blocked in every region"),
        "a block is not a stale client_id; refreshing it only costs a page load: {error}"
    );
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/tracks/"),
        tags(&[None]),
        "a track removed or blocked everywhere must not cost a read per region on every play"
    );
}

#[tokio::test]
async fn an_open_track_is_read_once_and_never_steered() {
    let world = World::default();
    world.open_in(&[None, Some("geo-us"), Some("geo-fr")]);
    let anon = anon_behind(&world).await;

    let src = anon
        .resolve_restricted(TRACK_URN, false, quality::SQ)
        .await
        .expect("soundcloud answers")
        .expect("the track has a full encrypted transcoding");

    assert_eq!(src.token, "license");
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/tracks/"),
        tags(&[None])
    );
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/media/"),
        tags(&[None])
    );
}

#[tokio::test]
async fn a_resolve_that_lands_in_a_closed_region_is_retried_through_one_that_sees_the_track() {
    let world = World::default();
    world.open_in(&[None, Some("geo-us"), Some("geo-fr")]);
    world.resolves_in(&[Some("geo-us"), Some("geo-fr")]);
    let anon = anon_behind(&world).await;

    let src = anon
        .resolve_restricted(TRACK_URN, false, quality::SQ)
        .await
        .expect("a region that sees the track resolves it")
        .expect("the track has a full encrypted transcoding");

    assert_eq!(src.token, "license");
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/tracks/"),
        tags(&[None, Some("geo-us")])
    );
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/media/"),
        tags(&[None, Some("geo-us")]),
        "the meta came from a proxy that sees the track and the resolve from one that does \
         not; this 404 was half of the restricted failures on main"
    );

    anon.read_track(TRACK_ID).await.expect("the track is read");
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/tracks/"),
        tags(&[Some("geo-us")])
    );
}

#[tokio::test]
async fn a_track_that_resolves_nowhere_keeps_no_region() {
    let world = World::default();
    world.open_in(&[None, Some("geo-us"), Some("geo-fr")]);
    world.resolves_in(&[]);
    let anon = anon_behind(&world).await;

    let error = anon
        .resolve_restricted(TRACK_URN, false, quality::SQ)
        .await
        .err()
        .expect("nothing resolves");
    assert!(error.to_string().contains("404"), "{error}");
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/media/"),
        tags(&[None, Some("geo-us")]),
        "one retry per play, not one per region"
    );

    world.take_asked("api-v2.soundcloud.com/tracks/");
    anon.read_track(TRACK_ID).await.expect("the track is read");
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/tracks/"),
        tags(&[None]),
        "a region that did not resolve the track must not pin it for a day"
    );
}

#[tokio::test]
async fn a_plain_copy_resolved_in_a_closed_region_is_fetched_through_one_that_sees_it() {
    let world = World::default();
    world.plain.store(true, Ordering::SeqCst);
    world.open_in(&[None, Some("geo-us"), Some("geo-fr")]);
    world.resolves_in(&[Some("geo-fr")]);
    let anon = anon_behind(&world).await;

    let found = anon.get_stream(TRACK_URN, quality::SQ).await;

    assert!(
        found.as_ref().is_ok_and(Option::is_none),
        "geo-us sees the meta but does not resolve, so this play has nothing"
    );
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/media/"),
        tags(&[None, None, Some("geo-us"), Some("geo-us")]),
        "each plain resolve keeps its one retry with a fresh client_id, in the same region"
    );

    world.resolves_in(&[Some("geo-us")]);
    let found = anon
        .get_stream(TRACK_URN, quality::SQ)
        .await
        .expect("soundcloud answers")
        .expect("the region that sees the track resolves the mp3");
    assert_eq!(found.quality, quality::SQ);
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/media/"),
        tags(&[Some("geo-us")])
    );
}

#[tokio::test]
async fn a_region_with_no_live_proxy_is_skipped_at_once_and_does_not_spoil_the_verdict() {
    let world = World::default();
    *world.absent.lock().unwrap() = tags(&[Some("geo-us")]);
    let anon = anon_behind(&world).await;

    let started = std::time::Instant::now();
    anon.read_track(TRACK_ID).await.expect("soundcloud answers");

    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "our proxy answers 502 at once for a tag no proxy carries; retrying it like a \
         soundcloud 5xx costs three seconds per missing region on every blocked play"
    );
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/tracks/"),
        tags(&[None, Some("geo-us"), Some("geo-fr")])
    );

    anon.read_track(TRACK_ID).await.expect("soundcloud answers");
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/tracks/"),
        tags(&[None]),
        "star sees the cloudflare workers as us and ca, so geo-fr is missing there for good; \
         a missing region must not keep a track blocked everywhere from being remembered"
    );
}

#[tokio::test]
async fn a_refused_abr_sq_falls_back_to_aac_96k_without_a_region_hunt() {
    let world = World::default();
    world.sq_only.store(true, Ordering::SeqCst);
    world.open_in(&[None, Some("geo-us"), Some("geo-fr")]);
    let anon = anon_behind(&world).await;

    let src = anon
        .resolve_restricted(TRACK_URN, false, quality::SQ)
        .await
        .expect("aac_96k resolves")
        .expect("the track has a full encrypted transcoding");

    assert_eq!(src.token, "license");
    let resolved: Vec<String> = world
        .asked
        .lock()
        .unwrap()
        .iter()
        .filter(|(target, _)| target.contains("/media/"))
        .map(|(target, _)| target.split('/').nth(5).unwrap_or("").to_owned())
        .collect();
    assert_eq!(resolved, ["abr_sq", "aac_96k"]);
    assert_eq!(
        world.take_asked("api-v2.soundcloud.com/tracks/"),
        tags(&[None]),
        "a 404 that one transcoding earns is not a region block"
    );
}
