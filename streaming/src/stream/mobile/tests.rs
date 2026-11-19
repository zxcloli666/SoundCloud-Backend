use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use backend_contracts::auth::token_fingerprint;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use uuid::Uuid;

use super::*;

const URN: &str = "soundcloud:tracks:7001";
const FULL_MS: i64 = 218_533;
const MEDIA: &str = "https://api-mobile.soundcloud.com/media/";
const PLAYLIST: &str = "https://cf-hls-media.sndcdn.com/playlist/0/218/a.128.mp3/playlist.m3u8";
const CDN_MEDIA: &str = "https://cf-hls-media.sndcdn.com/media/";
const INIT: &[u8] = b"\x00\x00\x00\x10ftypiso6mp41";
const OLD_TOKEN: &str = "old-token";
const NEW_TOKEN: &str = "new-token";

#[derive(Clone)]
struct Fake {
    track: Value,
    media: Media,
    segments: Vec<f64>,
    plan: &'static str,
    rejects: Option<&'static str>,
    asked: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone)]
enum Media {
    Playlist,
    To(&'static str),
    Status(StatusCode),
    Captcha,
}

impl Fake {
    fn new(track: Value) -> Self {
        Self {
            track,
            media: Media::Playlist,
            segments: vec![10.0; 22],
            plan: "none",
            rejects: None,
            asked: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn asked(&self, prefix: &str) -> usize {
        guard(&self.asked)
            .iter()
            .filter(|target| target.starts_with(prefix))
            .count()
    }
}

async fn answer(State(fake): State<Fake>, headers: HeaderMap) -> Response {
    let target = headers
        .get("x-target")
        .and_then(|value| STANDARD.decode(value.as_bytes()).ok())
        .and_then(|value| String::from_utf8(value).ok())
        .unwrap_or_default();
    guard(&fake.asked).push(target.clone());
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let to_soundcloud = target.starts_with(sc_mobile::GRAPH_URL) || target.starts_with(MEDIA);
    if to_soundcloud
        && fake
            .rejects
            .is_some_and(|stale| token == format!("OAuth {stale}"))
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if target.starts_with(sc_mobile::GRAPH_URL) {
        return axum::Json(json!({ "data": { "tracks": [fake.track] } })).into_response();
    }
    if target.starts_with(MEDIA) {
        return media(&fake);
    }
    if target.starts_with(PLAYLIST) {
        return playlist(&fake.segments).into_response();
    }
    if target.ends_with("init.mp4") {
        return INIT.into_response();
    }
    if let Some(segment) = target.strip_prefix(CDN_MEDIA) {
        return format!("segment {segment}").into_response();
    }
    StatusCode::NOT_FOUND.into_response()
}

fn media(fake: &Fake) -> Response {
    let plan = [("sc-mob-userplan", fake.plan)];
    match fake.media {
        Media::Playlist => {
            (StatusCode::FOUND, plan, [(header::LOCATION, PLAYLIST)]).into_response()
        }
        Media::To(location) => {
            (StatusCode::FOUND, plan, [(header::LOCATION, location)]).into_response()
        }
        Media::Status(status) => (status, plan, "{}").into_response(),
        Media::Captcha => (
            StatusCode::FORBIDDEN,
            r#"{"url":"https://geo.captcha-delivery.com/captcha/?initialCid=fake"}"#,
        )
            .into_response(),
    }
}

fn playlist(segments: &[f64]) -> String {
    let mut text = format!(
        "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-TARGETDURATION:10\n\
         #EXT-X-MAP:URI=\"{CDN_MEDIA}init.mp4\"\n"
    );
    for (index, seconds) in segments.iter().enumerate() {
        text.push_str(&format!(
            "#EXTINF:{seconds:.6},\n{CDN_MEDIA}{index}/a.m4s\n"
        ));
    }
    text.push_str("#EXT-X-ENDLIST\n");
    text
}

fn track(authorization: Value, transcodings: Vec<Value>) -> Value {
    json!({
        "urn": URN,
        "fullDuration": FULL_MS,
        "snipDuration": 30000,
        "authorization": authorization,
        "transcodings": transcodings,
    })
}

fn open_to_all() -> Value {
    json!({
        "monetizable": false,
        "snipped": false,
        "syncable": true,
        "subMidTier": false,
        "subHighTier": false,
        "policy": "ALLOW",
        "blocked": false,
        "monetizationModel": "NOT_APPLICABLE",
    })
}

fn go_plus_only(snipped: bool) -> Value {
    json!({
        "monetizable": true,
        "snipped": snipped,
        "syncable": false,
        "subMidTier": false,
        "subHighTier": true,
        "policy": "SNIP",
        "blocked": false,
        "monetizationModel": "SUB_HIGH_TIER",
    })
}

fn stream(preset: &str, quality: &str, mime: &str) -> Value {
    transcoding(preset, quality, mime, "stream/hls", FULL_MS + 1, false)
}

fn transcoding(
    preset: &str,
    quality: &str,
    mime: &str,
    path: &str,
    duration_ms: i64,
    snipped: bool,
) -> Value {
    json!({
        "url": format!("{MEDIA}{URN}/{preset}-uuid/{path}"),
        "uuid": format!("{preset}-uuid"),
        "preset": preset,
        "durationMs": duration_ms,
        "format": { "protocol": "hls", "mimeType": mime },
        "quality": quality,
        "snipped": snipped,
        "type": if snipped { "snip-preview" } else { "stream" },
    })
}

fn usual_track() -> Value {
    track(
        open_to_all(),
        vec![
            stream("aac_160k", "sq", "audio/mp4; codecs=\"mp4a.40.2\""),
            stream("aac_96k", "lq", "audio/mp4; codecs=\"mp4a.40.2\""),
            stream("abr_sq", "sq", "audio/mpegurl"),
            stream("mp3_0_1", "sq", "audio/mpeg"),
        ],
    )
}

fn session(plan: Option<&str>, fresh: bool) -> MobileSession {
    MobileSession {
        connection_id: Uuid::nil(),
        account: "501".to_owned(),
        device_id: "d".repeat(32),
        access_token: Secret::new(OLD_TOKEN.to_owned()),
        fresh,
        plan: plan.map(str::to_owned),
        app: MobileApp {
            client_id: "fake-client".to_owned(),
            client_secret: Secret::default(),
            app_id: "fake.app".to_owned(),
            app_version: "1.0".to_owned(),
            user_agent: "FakeAgent/1.0".to_owned(),
        },
    }
}

fn renewing(to: Option<&'static str>) -> (TokenRefresh, Arc<Mutex<Vec<Option<String>>>>) {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let seen = asked.clone();
    let refresh: TokenRefresh = Arc::new(move |_, rejected| {
        guard(&seen).push(rejected);
        Box::pin(async move { to.map(|token| Secret::new(token.to_owned())) })
    });
    (refresh, asked)
}

async fn mobile(fake: &Fake, refresh: TokenRefresh) -> Mobile {
    let app = Router::new().fallback(answer).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a local port");
    let address = listener.local_addr().expect("a local address");
    tokio::spawn(async move { axum::serve(listener, app).await });
    let http = sc_fingerprint::builder(None)
        .build()
        .expect("an http client");
    Mobile::new(refresh, http, format!("http://{address}/"), false, false)
}

async fn find_once(fake: &Fake, plan: Option<&str>, tier: &'static str) -> (Found, Link) {
    let (refresh, _) = renewing(None);
    let mobile = mobile(fake, refresh).await;
    let link = mobile.link(session(plan, true)).expect("a client");
    (mobile.find(&link, URN, tier).await, link)
}

fn outcome(found: &Found) -> Option<&'static str> {
    match found {
        Found::Miss(miss) => Some(*miss),
        Found::Clear(_) | Found::Encrypted(_) => None,
    }
}

#[tokio::test]
async fn a_free_account_gets_the_clear_aac_160k_whole() {
    let fake = Fake::new(usual_track());

    let (found, link) = find_once(&fake, Some("none"), quality::HQ).await;

    let Found::Clear((data, content_type, tier)) = found else {
        panic!("expected a clear stream, got {:?}", outcome(&found));
    };
    assert_eq!((content_type, tier), ("audio/mp4", quality::HQ));
    assert!(data.starts_with(INIT));
    assert!(data.ends_with(b"segment 21/a.m4s"));
    assert!(!link.closed.load(Ordering::Relaxed));
    assert!(
        guard(&link.unsaved_plan).is_none(),
        "the plan did not change"
    );
}

#[tokio::test]
async fn sq_skips_the_adaptive_and_the_low_preset() {
    let fake = Fake::new(usual_track());

    let (found, _) = find_once(&fake, Some("none"), quality::SQ).await;

    assert!(matches!(
        found,
        Found::Clear((_, "audio/mpeg", quality::SQ))
    ));
    assert_eq!(fake.asked(&format!("{MEDIA}{URN}/mp3_0_1-uuid/")), 1);
}

#[tokio::test]
async fn a_plan_seen_in_a_reply_is_kept_once_and_opens_uq() {
    let mut fake = Fake::new(usual_track());
    fake.plan = "high_tier";
    let (refresh, _) = renewing(None);
    let mobile = mobile(&fake, refresh).await;
    let link = mobile.link(session(Some("none"), true)).expect("a client");
    assert!(!link.serves(quality::UQ));

    let found = mobile.find(&link, URN, quality::HQ).await;

    assert!(matches!(found, Found::Clear(_)));
    assert_eq!(
        guard(&link.unsaved_plan).take().as_deref(),
        Some("high_tier")
    );
    assert!(link.serves(quality::UQ));
    assert!(link.serves_uq());
}

#[tokio::test]
async fn a_tier_the_track_lacks_is_a_miss_that_keeps_the_link_and_its_lookup() {
    let fake = Fake::new(track(
        open_to_all(),
        vec![stream("mp3_0_1", "sq", "audio/mpeg")],
    ));
    let (refresh, _) = renewing(None);
    let mobile = mobile(&fake, refresh).await;
    let link = mobile.link(session(Some("none"), true)).expect("a client");

    let hq = mobile.find(&link, URN, quality::HQ).await;
    let sq = mobile.find(&link, URN, quality::SQ).await;

    assert_eq!(outcome(&hq), Some(MISSED));
    assert!(matches!(sq, Found::Clear(_)));
    assert_eq!(
        fake.asked(sc_mobile::GRAPH_URL),
        1,
        "one lookup serves every tier"
    );
}

#[tokio::test]
async fn a_refused_media_moves_on_at_once_and_closes_the_link() {
    for status in [StatusCode::FORBIDDEN, StatusCode::NOT_FOUND] {
        let mut fake = Fake::new(usual_track());
        fake.media = Media::Status(status);

        let (found, link) = find_once(&fake, Some("none"), quality::HQ).await;

        assert_eq!(outcome(&found), Some(REFUSED), "{status}");
        assert!(!link.serves(quality::SQ), "{status}");
        assert_eq!(fake.asked(MEDIA), 1, "{status} is not retried");
    }
}

#[tokio::test]
async fn a_captcha_is_a_miss_without_a_retry() {
    let mut fake = Fake::new(usual_track());
    fake.media = Media::Captcha;

    let (found, link) = find_once(&fake, Some("none"), quality::HQ).await;

    assert_eq!(outcome(&found), Some(CAPTCHA));
    assert!(!link.serves(quality::SQ));
    assert_eq!(fake.asked(MEDIA), 1);
}

#[tokio::test]
async fn a_rejected_token_is_renewed_by_core_and_the_call_tried_once_more() {
    let mut fake = Fake::new(usual_track());
    fake.rejects = Some(OLD_TOKEN);
    let (refresh, asked) = renewing(Some(NEW_TOKEN));
    let mobile = mobile(&fake, refresh).await;
    let link = mobile.link(session(Some("none"), true)).expect("a client");

    let found = mobile.find(&link, URN, quality::HQ).await;

    assert!(matches!(found, Found::Clear(_)), "{:?}", outcome(&found));
    assert_eq!(*guard(&asked), vec![Some(token_fingerprint(OLD_TOKEN))]);
    assert_eq!(fake.asked(sc_mobile::GRAPH_URL), 2);
    assert_eq!(
        fake.asked(MEDIA),
        1,
        "the new token is kept for the next call"
    );
}

#[tokio::test]
async fn a_token_core_cannot_renew_ends_the_link() {
    for renewed in [None, Some(OLD_TOKEN)] {
        let mut fake = Fake::new(usual_track());
        fake.rejects = Some(OLD_TOKEN);
        let (refresh, _) = renewing(renewed);
        let mobile = mobile(&fake, refresh).await;
        let link = mobile.link(session(Some("none"), true)).expect("a client");

        let found = mobile.find(&link, URN, quality::HQ).await;

        assert_eq!(outcome(&found), Some(REFUSED), "{renewed:?}");
        assert!(!link.serves(quality::SQ));
        assert_eq!(fake.asked(sc_mobile::GRAPH_URL), 1, "{renewed:?}");
    }
}

#[tokio::test]
async fn a_stale_token_is_renewed_before_the_first_call() {
    let mut fake = Fake::new(usual_track());
    fake.rejects = Some(OLD_TOKEN);
    let (refresh, asked) = renewing(Some(NEW_TOKEN));
    let mobile = mobile(&fake, refresh).await;
    let link = mobile.link(session(Some("none"), false)).expect("a client");

    let found = mobile.find(&link, URN, quality::HQ).await;

    assert!(matches!(found, Found::Clear(_)));
    assert_eq!(*guard(&asked), vec![None]);
    assert_eq!(fake.asked(sc_mobile::GRAPH_URL), 1);
}

#[tokio::test]
async fn a_track_snipped_for_this_account_is_a_preview_before_any_media_call() {
    let fake = Fake::new(track(
        go_plus_only(true),
        vec![transcoding(
            "mp3_1_0",
            "sq",
            "audio/mpeg",
            "snippet/hls",
            20_000,
            true,
        )],
    ));

    let (found, link) = find_once(&fake, Some("none"), quality::SQ).await;

    assert_eq!(outcome(&found), Some(PREVIEW));
    assert!(!link.serves(quality::SQ));
    assert_eq!(fake.asked(MEDIA), 0);
}

#[tokio::test]
async fn a_snip_policy_is_a_preview_unless_the_plan_opens_the_track() {
    let fake = Fake::new(track(
        go_plus_only(false),
        vec![stream("aac_160k", "sq", "audio/mp4; codecs=\"mp4a.40.2\"")],
    ));

    for plan in [None, Some("none"), Some("mid_tier"), Some("student_tier")] {
        let (found, _) = find_once(&fake, plan, quality::HQ).await;
        assert_eq!(outcome(&found), Some(PREVIEW), "{plan:?}");
    }
    let (found, _) = find_once(&fake, Some("high_tier"), quality::HQ).await;
    assert!(matches!(found, Found::Clear(_)), "{:?}", outcome(&found));
}

#[tokio::test]
async fn every_kind_of_preview_transcoding_is_a_miss() {
    let mime = "audio/mpeg";
    for (kind, previews) in [
        (
            "snipped",
            transcoding("mp3_0_1", "sq", mime, "stream/hls", FULL_MS, true),
        ),
        (
            "preview url",
            transcoding("mp3_0_1", "sq", mime, "preview/hls", FULL_MS, false),
        ),
        (
            "snippet url",
            transcoding("mp3_0_1", "sq", mime, "snippet/hls", FULL_MS, false),
        ),
        (
            "short duration",
            transcoding("mp3_0_1", "sq", mime, "stream/hls", 30_000, false),
        ),
    ] {
        let fake = Fake::new(track(open_to_all(), vec![previews]));

        let (found, _) = find_once(&fake, Some("none"), quality::SQ).await;

        assert_eq!(outcome(&found), Some(PREVIEW), "{kind}");
        assert_eq!(fake.asked(MEDIA), 0, "{kind}");
    }
}

#[tokio::test]
async fn a_full_transcoding_is_chosen_over_a_preview_beside_it() {
    let fake = Fake::new(track(
        open_to_all(),
        vec![
            transcoding("mp3_0_1", "sq", "audio/mpeg", "snippet/hls", 20_000, true),
            stream("opus_0_0", "sq", "audio/ogg; codecs=\"opus\""),
        ],
    ));

    let (found, _) = find_once(&fake, Some("none"), quality::SQ).await;

    assert!(matches!(found, Found::Clear((_, "audio/ogg", _))));
}

#[tokio::test]
async fn a_playlist_shorter_than_the_track_is_a_preview() {
    let mut fake = Fake::new(usual_track());
    fake.segments = vec![1.985272, 2.977908, 4.963180];

    let (found, link) = find_once(&fake, Some("none"), quality::HQ).await;

    assert_eq!(outcome(&found), Some(PREVIEW));
    assert!(!link.serves(quality::SQ));
    assert_eq!(
        fake.asked(CDN_MEDIA),
        0,
        "no segment of a preview is fetched"
    );
}

#[tokio::test]
async fn a_redirect_to_a_preview_is_a_preview() {
    let mut fake = Fake::new(usual_track());
    fake.media = Media::To("https://cf-preview-media.sndcdn.com/preview/0/30/a.128.mp3");

    let (found, _) = find_once(&fake, Some("none"), quality::HQ).await;

    assert_eq!(outcome(&found), Some(PREVIEW));
}

#[tokio::test]
async fn a_track_without_its_full_length_is_not_trusted() {
    let mut track = usual_track();
    track["fullDuration"] = Value::Null;
    let fake = Fake::new(track);

    let (found, _) = find_once(&fake, Some("none"), quality::HQ).await;

    assert_eq!(outcome(&found), Some(PREVIEW));
    assert_eq!(fake.asked(MEDIA), 0);
}

#[tokio::test]
async fn an_encrypted_transcoding_waits_for_a_node_that_can_decrypt() {
    let mut encrypted = stream("aac_160k", "sq", "audio/mp4; codecs=\"mp4a.40.2\"");
    encrypted["format"]["protocol"] = json!("ctr-encrypted-hls");
    let fake = Fake::new(track(open_to_all(), vec![encrypted]));

    let (found, _) = find_once(&fake, Some("none"), quality::HQ).await;

    assert_eq!(outcome(&found), Some(MISSED));
    assert_eq!(fake.asked(MEDIA), 0);
}

#[tokio::test]
async fn a_blocked_track_is_refused() {
    let mut authorization = open_to_all();
    authorization["policy"] = json!("BLOCK");
    let fake = Fake::new(track(
        authorization,
        vec![stream("mp3_0_1", "sq", "audio/mpeg")],
    ));

    let (found, _) = find_once(&fake, Some("none"), quality::SQ).await;

    assert_eq!(outcome(&found), Some(REFUSED));
}
