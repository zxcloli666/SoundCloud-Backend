use backend_contracts::admin_video::VideoSource;
use wreq::header::HeaderMap;

use super::mock::{Answer, Mock};
use super::{Checker, SpotifyKey, Verdict, totp};

const CIPHER: [u8; 3] = [41, 41, 41];

fn checker(mock: &std::sync::Arc<Mock>) -> Checker<std::sync::Arc<Mock>> {
    Checker::new(mock.clone(), SpotifyKey::new(61, &CIPHER))
}

fn header(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

fn rejected(verdict: &Verdict) -> bool {
    matches!(verdict, Verdict::Rejected(_))
}

fn unavailable(verdict: &Verdict) -> bool {
    matches!(verdict, Verdict::Unavailable(_))
}

#[tokio::test]
async fn yandex_accepts_a_token_with_an_account_and_asks_from_a_russian_exit() {
    let mock = Mock::new();
    mock.route(
        "https://api.music.yandex.net/account/status",
        Answer::Body(r#"{"result":{"account":{"uid":42,"login":"x"}}}"#.to_owned()),
    );

    let verdict = checker(&mock).check(VideoSource::Yandex, " tok \n").await;

    assert_eq!(verdict, Verdict::Valid);
    let requests = mock.requests();
    let (_, headers) = &requests[0];
    assert_eq!(header(headers, "authorization"), "OAuth tok");
    assert_eq!(header(headers, "x-egress-tag"), "geo-ru");
}

#[tokio::test]
async fn yandex_rejects_only_what_the_service_rejects() {
    let url = "https://api.music.yandex.net/account/status";
    let cases = [
        (Answer::Status(401), true),
        (Answer::Body(r#"{"result":{}}"#.to_owned()), true),
        (Answer::Status(403), false),
        (Answer::Status(451), false),
        (Answer::Down, false),
        (Answer::Body("<html>".to_owned()), false),
    ];
    for (answer, dead) in cases {
        let mock = Mock::new();
        mock.route(url, answer);

        let verdict = checker(&mock).check(VideoSource::Yandex, "tok").await;

        assert_eq!(rejected(&verdict), dead, "{verdict:?}");
        assert_eq!(unavailable(&verdict), !dead, "{verdict:?}");
    }
}

fn spotify_mock(token: Answer) -> std::sync::Arc<Mock> {
    let mock = Mock::new();
    mock.route(
        "https://open.spotify.com/api/server-time",
        Answer::Body(r#"{"serverTime":1700000000}"#.to_owned()),
    );
    mock.route("https://open.spotify.com/api/token", token);
    mock
}

#[tokio::test]
async fn spotify_signs_the_token_request_with_a_totp_of_the_server_time() {
    let mock = spotify_mock(Answer::Body(
        r#"{"accessToken":"abc","isAnonymous":false}"#.to_owned(),
    ));

    let verdict = checker(&mock).check(VideoSource::Spotify, "dc-value").await;

    assert_eq!(verdict, Verdict::Valid);
    let requests = mock.requests();
    let (url, headers) = &requests[1];
    let key = SpotifyKey::new(61, &CIPHER);
    let code = totp::code(&key.secret, 1_700_000_000).unwrap_or_default();
    assert!(url.contains(&format!("totp={code}")), "{url}");
    assert!(url.contains("totpVer=61"), "{url}");
    assert_eq!(header(headers, "cookie"), "sp_dc=dc-value");
    assert_eq!(header(&requests[0].1, "cookie"), "");
}

#[tokio::test]
async fn spotify_kills_an_account_only_when_the_token_is_anonymous() {
    let anonymous = spotify_mock(Answer::Body(
        r#"{"accessToken":"abc","isAnonymous":true}"#.to_owned(),
    ));
    let forbidden = spotify_mock(Answer::Status(403));
    let empty = spotify_mock(Answer::Body("{}".to_owned()));

    let anonymous = checker(&anonymous).check(VideoSource::Spotify, "dc").await;
    let forbidden = checker(&forbidden).check(VideoSource::Spotify, "dc").await;
    let empty = checker(&empty).check(VideoSource::Spotify, "dc").await;

    assert!(rejected(&anonymous), "{anonymous:?}");
    assert!(unavailable(&forbidden), "{forbidden:?}");
    assert!(unavailable(&empty), "{empty:?}");
}

#[tokio::test]
async fn spotify_without_a_clock_is_unavailable_and_never_asks_for_a_token() {
    let mock = Mock::new();

    let verdict = checker(&mock).check(VideoSource::Spotify, "dc").await;

    assert!(unavailable(&verdict), "{verdict:?}");
    assert_eq!(mock.requests().len(), 1);
}

const COOKIES: &str = "# Netscape HTTP Cookie File\n\
.youtube.com\tTRUE\t/\tTRUE\t1893456000\tYSC\tabc\n\
#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1893456000\t__Secure-3PSID\tsid-value\n\
.google.com\tTRUE\t/\tTRUE\t1893456000\tNID\tnid-value\n\
.example.com\tTRUE\t/\tTRUE\t1893456000\ttracker\tno\n";

#[tokio::test]
async fn youtube_reads_the_login_marker_and_sends_only_google_cookies() {
    let mock = Mock::new();
    mock.route(
        "https://www.youtube.com/",
        Answer::Body(r#"ytcfg.set({"LOGGED_IN":true,"X":1})"#.to_owned()),
    );

    let verdict = checker(&mock).check(VideoSource::Youtube, COOKIES).await;

    assert_eq!(verdict, Verdict::Valid);
    let cookie = header(&mock.requests()[0].1, "cookie");
    assert_eq!(cookie, "YSC=abc; __Secure-3PSID=sid-value; NID=nid-value");
}

#[tokio::test]
async fn youtube_tells_signed_out_from_an_unreadable_page() {
    let cases = [
        (r#"{"LOGGED_IN":false}"#, true),
        ("<html>consent</html>", false),
    ];
    for (page, dead) in cases {
        let mock = Mock::new();
        mock.route("https://www.youtube.com/", Answer::Body(page.to_owned()));

        let verdict = checker(&mock).check(VideoSource::Youtube, COOKIES).await;

        assert_eq!(rejected(&verdict), dead, "{verdict:?}");
        assert_eq!(unavailable(&verdict), !dead, "{verdict:?}");
    }
}

#[tokio::test]
async fn youtube_rejects_a_file_without_a_session_before_any_request() {
    let mock = Mock::new();
    let file = ".youtube.com\tTRUE\t/\tTRUE\t1893456000\tYSC\tabc\n";

    let verdict = checker(&mock).check(VideoSource::Youtube, file).await;

    assert!(rejected(&verdict), "{verdict:?}");
    assert!(mock.requests().is_empty());
}
