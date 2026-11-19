use std::collections::HashMap;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde_json::json;

use super::*;
use crate::comments;
use crate::me;

fn dummy_app() -> MobileApp {
    MobileApp {
        client_id: "dummy-client".to_owned(),
        client_secret: Secret::new("dummy-secret".to_owned()),
        app_id: "dummy-app".to_owned(),
        app_version: "dummy-version".to_owned(),
        user_agent: "dummy-agent".to_owned(),
    }
}

fn signed_in() -> Caller {
    Caller::user("dummy-device", Secret::new("dummy-token".to_owned()))
}

fn header<'a>(prepared: &'a Prepared, name: &str) -> Option<&'a str> {
    prepared
        .headers
        .get(name)
        .and_then(|value| value.to_str().ok())
}

fn query(url: &Url) -> HashMap<String, String> {
    url.query_pairs().into_owned().collect()
}

fn prepared(caller: &Caller, request: &MobileRequest) -> Prepared {
    match prepare(&dummy_app(), caller, request) {
        Ok(prepared) => prepared,
        Err(error) => panic!("the request does not prepare: {error}"),
    }
}

#[test]
fn every_api_request_carries_the_client_id_and_the_client_headers() {
    let request = comments::list("soundcloud:tracks:1").with_query("limit", "5");
    let prepared = prepared(&signed_in(), &request);

    assert_eq!(prepared.method, Method::GET);
    assert_eq!(
        prepared.target.host_str(),
        Some("api-mobile.soundcloud.com")
    );
    assert_eq!(
        prepared.target.path(),
        "/tracks/soundcloud:tracks:1/comments"
    );
    let query = query(&prepared.target);
    assert_eq!(
        query.get("client_id").map(String::as_str),
        Some("dummy-client")
    );
    assert_eq!(query.get("limit").map(String::as_str), Some("5"));
    assert_eq!(
        header(&prepared, "authorization"),
        Some("OAuth dummy-token")
    );
    assert_eq!(header(&prepared, "udid"), Some("dummy-device"));
    assert_eq!(header(&prepared, "app-version"), Some("dummy-version"));
    assert_eq!(header(&prepared, "user-agent"), Some("dummy-agent"));
    assert_eq!(header(&prepared, "accept-encoding"), Some("identity"));
    assert!(prepared.body.is_none());
}

#[test]
fn graphql_carries_the_client_id_too_and_an_anonymous_caller_sends_no_authorization() {
    let request = MobileRequest::query("query Me { me { urn } }", json!({}));
    let prepared = prepared(&Caller::device("dummy-device"), &request);

    assert_eq!(prepared.method, Method::POST);
    assert_eq!(prepared.target.host_str(), Some("graph.soundcloud.com"));
    assert_eq!(
        query(&prepared.target).get("client_id").map(String::as_str),
        Some("dummy-client")
    );
    assert_eq!(header(&prepared, "authorization"), None);
    assert_eq!(header(&prepared, "udid"), Some("dummy-device"));
    assert_eq!(
        header(&prepared, "content-type"),
        Some("application/json; charset=utf-8")
    );
}

#[test]
fn the_token_request_authenticates_the_client_by_header_and_keeps_the_secret_out_of_the_body() {
    let request =
        MobileRequest::token("grant_type=refresh_token&client_id=dummy-client".to_owned());
    let prepared = prepared(&Caller::device("dummy-device"), &request);

    assert_eq!(
        prepared.target.as_str(),
        "https://api-auth.soundcloud.com/oauth/token"
    );
    assert_eq!(
        header(&prepared, "authorization"),
        Some(client_authorization(&dummy_app()).expose().as_str())
    );
    assert_eq!(
        header(&prepared, "content-type"),
        Some("application/x-www-form-urlencoded")
    );
    let body = prepared.body.clone().unwrap_or_default();
    assert!(!String::from_utf8_lossy(&body).contains("dummy-secret"));
    assert!(!prepared.target.as_str().contains("dummy-secret"));
}

#[test]
fn a_proxied_request_goes_to_the_proxy_with_the_target_in_x_target() {
    let Ok(client) = MobileClient::new(dummy_app(), Egress::Direct) else {
        panic!("the client does not build");
    };
    let Ok(proxy) = Url::parse("http://proxy.internal:8080/") else {
        panic!("the proxy url does not parse");
    };
    let prepared = prepared(&signed_in(), &me::me());

    let Ok(proxied) = transport::outgoing(&client.http, &prepared, Route::Proxy(&proxy)).build()
    else {
        panic!("the proxied request does not build");
    };
    let target = proxied
        .headers()
        .get("x-target")
        .and_then(|value| STANDARD.decode(value.as_bytes()).ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());

    assert_eq!(proxied.uri().host(), Some("proxy.internal"));
    assert_eq!(target.as_deref(), Some(prepared.target.as_str()));
    assert!(proxied.headers().get("authorization").is_some());

    let Ok(direct) = transport::outgoing(&client.http, &prepared, Route::Direct).build() else {
        panic!("the direct request does not build");
    };
    assert_eq!(direct.uri().host(), Some("api-mobile.soundcloud.com"));
    assert!(direct.headers().get("x-target").is_none());
}

#[test]
fn only_reads_and_queries_get_a_second_send() {
    let Ok(proxy) = Url::parse("http://proxy.internal:8080/") else {
        panic!("the proxy url does not parse");
    };
    let with_fallback = Egress::ProxyThenDirect(proxy.clone());
    let proxy_only = Egress::Proxy(proxy.clone());

    assert_eq!(
        second_route(Kind::Read, &with_fallback),
        Some(Route::Direct)
    );
    assert_eq!(second_route(Kind::Read, &proxy_only), None);
    assert_eq!(second_route(Kind::Read, &Egress::Direct), None);
    assert_eq!(
        second_route(Kind::Query, &with_fallback),
        Some(Route::Proxy(&proxy))
    );
    for kind in [Kind::Write, Kind::Mutation, Kind::Token] {
        assert_eq!(second_route(kind, &with_fallback), None, "{kind:?}");
    }
}

#[test]
fn a_second_send_follows_only_a_transport_failure_or_a_server_error() {
    let reply = |status| MobileReply {
        status,
        plan: None,
        location: None,
        body: Bytes::new(),
    };

    assert!(worth_another_send(&Ok(reply(502))));
    assert!(worth_another_send(&Err(MobileError::Transport(
        "reset".to_owned()
    ))));
    assert!(!worth_another_send(&Ok(reply(429))));
    assert!(!worth_another_send(&Ok(reply(404))));
    assert!(!worth_another_send(&Ok(reply(200))));
    assert!(!worth_another_send(&Err(MobileError::TooLarge(1))));
}

#[test]
fn a_caller_prints_neither_its_token_nor_its_device() {
    let printed = format!("{:?}", signed_in());

    assert!(!printed.contains("dummy-token"), "{printed}");
    assert!(!printed.contains("dummy-device"), "{printed}");
}
