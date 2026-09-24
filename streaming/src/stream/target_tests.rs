use super::*;

const INSIDE: &[&str] = &[
    "https://127.0.0.1/x",
    "https://127.1.2.3/x",
    "https://localhost/x",
    "https://LOCALHOST./x",
    "https://storage/x",
    "https://minio.internal/x",
    "https://pg.local/x",
    "https://10.1.2.3/x",
    "https://172.16.9.9/x",
    "https://192.168.1.1/x",
    "https://169.254.169.254/latest/meta-data/",
    "https://100.100.1.1/x",
    "https://0.0.0.0/x",
    "https://[::1]/x",
    "https://[fd00::1]/x",
    "https://[fe80::1]/x",
    "https://[::ffff:127.0.0.1]/x",
];

#[test]
fn nothing_on_our_own_network_is_a_place_a_track_can_send_us() {
    for address in INSIDE {
        assert_eq!(
            public_media(address),
            None,
            "{address} is reachable only from inside our network, so a url a stranger put in \
             a soundcloud answer must never turn into a request to it"
        );
    }
}

#[test]
fn a_real_cdn_address_is_still_fetched() {
    for address in [
        "https://cf-hls-media.sndcdn.com/media/0/1/2.m3u8",
        "https://cf-media.sndcdn.com/abc.128.mp3",
        "https://api-v2.soundcloud.com/media/soundcloud:tracks:42/x/stream/hls",
        "https://8.8.8.8/x",
    ] {
        assert!(
            public_media(address).is_some(),
            "{address} is an ordinary public address; refusing it would stop playback \
             everywhere and this guard would still look green"
        );
    }
}

#[test]
fn a_url_that_hides_its_real_host_behind_credentials_is_refused() {
    for address in [
        "https://api-v2.soundcloud.com@evil.example/x",
        "https://api-v2.soundcloud.com:pw@127.0.0.1/x",
        "https://user@127.0.0.1/x",
    ] {
        assert_eq!(public_media(address), None, "{address} names two hosts");
        assert_eq!(soundcloud_api(address), None, "{address} names two hosts");
        assert_eq!(soundcloud_page(address), None, "{address} names two hosts");
    }
}

#[test]
fn only_soundcloud_itself_is_a_place_we_show_our_session_to() {
    for page in [
        "https://soundcloud.com/artist/track",
        "https://m.soundcloud.com/artist/track",
        "https://on.soundcloud.com/abcd",
    ] {
        assert!(page.len() < MAX_URL_LEN);
        assert!(
            soundcloud_page(page).is_some(),
            "{page} is where a permalink actually points"
        );
    }
    for elsewhere in [
        "https://soundcloud.com.evil.example/x",
        "https://evilsoundcloud.com/x",
        "https://cf-media.sndcdn.com/x",
        "http://soundcloud.com/x",
        "https://api-v2.soundcloud.com/tracks/42",
    ] {
        assert_eq!(
            soundcloud_page(elsewhere),
            None,
            "{elsewhere} would receive the listener's live soundcloud cookies"
        );
    }
}

#[test]
fn a_transcoding_is_only_ever_resolved_against_soundclouds_own_api() {
    assert!(
        soundcloud_api("https://api-v2.soundcloud.com/media/soundcloud:tracks:42/x/stream/hls")
            .is_some()
    );
    for elsewhere in [
        "https://cf-media.sndcdn.com/x",
        "https://api-v2.soundcloud.com.evil.example/x",
        "http://api-v2.soundcloud.com/x",
    ] {
        assert_eq!(
            soundcloud_api(elsewhere),
            None,
            "{elsewhere} is not the api"
        );
    }
}

#[test]
fn a_scheme_that_is_not_the_web_is_refused_everywhere() {
    for address in [
        "file:///etc/passwd",
        "gopher://127.0.0.1:70/x",
        "ftp://example.com/x",
        "data:text/plain,hello",
    ] {
        assert_eq!(
            public_media(address),
            None,
            "{address} is not an http fetch"
        );
    }
}

#[test]
fn an_address_too_long_to_read_is_refused_before_it_is_parsed() {
    let long = format!("https://example.com/{}", "a".repeat(MAX_URL_LEN));
    assert_eq!(public_media(&long), None);
}

#[test]
fn the_name_we_write_in_a_log_carries_no_capability() {
    let carrying = "https://api-v2.soundcloud.com/media/soundcloud:tracks:42/x/stream/hls\
                    ?client_id=CID9&track_authorization=eyJhbGciOi.SECRET.SIG\
                    &secret_token=s-Abc123#fragment";

    let written = named_without_secrets(carrying);

    for secret in [
        "CID9",
        "SECRET",
        "s-Abc123",
        "client_id",
        "track_authorization",
    ] {
        assert!(
            !written.contains(secret),
            "a failing fetch is logged at debug on every retry, so {secret} would be written \
             to disk on every flaky segment: {written}"
        );
    }
    assert_eq!(
        written, "https://api-v2.soundcloud.com/media/soundcloud:tracks:42/x/stream/hls",
        "the name must still say which resource failed, or the log stops being usable"
    );
}

#[test]
fn a_name_we_cannot_read_is_still_not_written_out_verbatim() {
    assert_eq!(
        named_without_secrets("not a url at all ?client_id=CID9"),
        "<unreadable url>",
        "falling back to the raw string would put the capability back in the log"
    );
}
