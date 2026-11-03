use super::{Mode, RoomsConfig};

const API: &str = "https://api.main.int.scnative.dev:8443";

#[test]
fn unset_means_local_with_no_handoff() {
    let config = RoomsConfig::parse("", None, "");
    assert_eq!(config.mode, Mode::Local);
    assert!(config.handoff_api().is_none());
}

#[test]
fn proxy_keeps_the_api_and_never_hands_off() {
    let config = RoomsConfig::parse("proxy", Some(API), "");
    assert_eq!(config.mode, Mode::Proxy);
    assert_eq!(
        config.api.as_ref().map(url::Url::as_str),
        Some("https://api.main.int.scnative.dev:8443/")
    );
    assert!(config.handoff_api().is_none());
}

#[test]
fn local_with_an_api_hands_off_with_the_token() {
    let config = RoomsConfig::parse("local", Some(API), " secret ");
    assert_eq!(config.mode, Mode::Local);
    assert!(config.handoff_api().is_some());
    assert_eq!(config.token.expose(), "secret");
}

#[test]
fn an_empty_api_is_no_api() {
    let config = RoomsConfig::parse("local", Some("  "), "");
    assert!(config.handoff_api().is_none());
}

#[test]
#[should_panic(expected = "ROOMS_MODE must be proxy or local")]
fn an_unknown_mode_stops_the_boot() {
    RoomsConfig::parse("both", Some(API), "t");
}

#[test]
#[should_panic(expected = "needs ROOMS_API")]
fn proxy_without_an_api_stops_the_boot() {
    RoomsConfig::parse("proxy", None, "t");
}

#[test]
#[should_panic(expected = "INTERNAL_TOKEN")]
fn a_handoff_without_the_token_stops_the_boot() {
    RoomsConfig::parse("local", Some(API), "");
}

#[test]
#[should_panic(expected = "ROOMS_API must be an http or https URL")]
fn a_non_http_api_stops_the_boot() {
    RoomsConfig::parse("proxy", Some("ftp://x"), "");
}
