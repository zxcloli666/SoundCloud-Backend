use sc_mobile::{Kind, MobileRequest};
use serde_json::{Value, json};

use super::super::actions::{ActionError, Remote};
use super::tests::claimed;
use super::{build, carries_only_sent_fields, shape};

const COMMENT: &str = "soundcloud:comments:9";
const TRACK: &str = "soundcloud:tracks:1";

fn path(request: &MobileRequest) -> Option<String> {
    request.url().ok().map(|url| url.path().to_owned())
}

fn body(request: &MobileRequest) -> Option<Value> {
    request
        .body()
        .and_then(|body| serde_json::from_slice(body).ok())
}

fn created(payload: Value) -> MobileRequest {
    match build(&claimed("playlist_create", "new:1", Some(payload))) {
        Ok(request) => request,
        Err(error) => panic!("a playlist create must build: {error}"),
    }
}

fn mapped(payload: Value, reply: Value) -> Result<Value, ActionError> {
    let mutation = claimed("playlist_create", "new:1", Some(payload));
    match shape(&mutation, Remote::Written(reply))? {
        Remote::Written(value) => Ok(value),
        Remote::TargetMissing => Err(ActionError::InvalidRemoteResult("closed")),
    }
}

#[test]
fn a_comment_like_is_a_graph_mutation_on_the_comment_under_its_track() {
    let payload = Some(json!({ "parent_urn": TRACK }));
    let (Ok(like), Ok(unlike)) = (
        build(&claimed("comment_like", COMMENT, payload.clone())),
        build(&claimed("comment_unlike", COMMENT, payload)),
    ) else {
        panic!("a comment like must build");
    };

    assert_eq!(like.kind(), Kind::Mutation);
    assert_eq!(unlike.kind(), Kind::Mutation);
    let query = |request: &MobileRequest| {
        body(request)
            .and_then(|body| body["query"].as_str().map(str::to_owned))
            .unwrap_or_default()
    };
    assert!(query(&like).contains("upsertInteraction"));
    assert!(query(&unlike).contains("removeInteraction"));
    for request in [&like, &unlike] {
        assert_eq!(
            body(request).map(|body| body["variables"].clone()),
            Some(json!({
                "input": {
                    "targetUrn": COMMENT,
                    "parentUrn": TRACK,
                    "interactionTypeUrn": "sc:interactiontype:reaction",
                    "interactionTypeValueUrn": "sc:interactiontypevalue:like",
                }
            }))
        );
    }
}

#[test]
fn a_comment_like_without_a_comment_urn_or_a_track_parent_is_refused() {
    let with_parent = Some(json!({ "parent_urn": TRACK }));
    for mutation in [
        claimed("comment_like", "soundcloud:tracks:9", with_parent),
        claimed("comment_like", COMMENT, None),
        claimed("comment_unlike", COMMENT, Some(json!({}))),
        claimed(
            "comment_unlike",
            COMMENT,
            Some(json!({ "parent_urn": "soundcloud:users:1" })),
        ),
    ] {
        assert!(matches!(
            build(&mutation),
            Err(ActionError::InvalidPayload(_))
        ));
    }
}

#[test]
fn a_track_like_posts_the_track_to_the_likes_endpoints() {
    let (Ok(like), Ok(unlike)) = (
        build(&claimed("like_track", TRACK, None)),
        build(&claimed("unlike_track", TRACK, None)),
    ) else {
        panic!("a track like must build");
    };

    assert_eq!(path(&like).as_deref(), Some("/likes/tracks/create"));
    assert_eq!(path(&unlike).as_deref(), Some("/likes/tracks/delete"));
    for request in [&like, &unlike] {
        assert_eq!(request.method().as_str(), "POST");
        assert_eq!(
            body(request),
            Some(json!({ "likes": [{ "target_urn": TRACK }] }))
        );
    }
    assert!(build(&claimed("like_track", "42", None)).is_err());
}

#[test]
fn a_playlist_create_carries_the_title_the_sharing_and_the_tracks_in_the_client_shape() {
    let private = created(json!({"playlist": {
        "title": " Mix ",
        "sharing": "private",
        "tracks": [{"urn": "soundcloud:tracks:5"}, {"id": 6}, "7", 5]
    }}));
    let public = created(json!({"playlist": {"title": "Open"}}));

    assert_eq!(private.method().as_str(), "POST");
    assert_eq!(path(&private).as_deref(), Some("/playlists"));
    assert_eq!(
        body(&private),
        Some(json!({
            "playlist": {"title": "Mix", "public": false},
            "track_urns": ["soundcloud:tracks:5", "soundcloud:tracks:6", "soundcloud:tracks:7"],
        }))
    );
    assert_eq!(
        body(&public),
        Some(json!({"playlist": {"title": "Open", "public": true}, "track_urns": []}))
    );
}

#[test]
fn a_playlist_create_without_a_title_or_with_a_broken_track_is_refused() {
    for payload in [
        json!({"playlist": {"sharing": "private"}}),
        json!({"playlist": {"title": "  "}}),
        json!({"playlist": {"title": "x", "tracks": ["abc"]}}),
        json!({"playlist": {"title": "x", "tracks": {}}}),
        json!({}),
    ] {
        assert!(matches!(
            build(&claimed("playlist_create", "new:1", Some(payload))),
            Err(ActionError::InvalidPayload(_))
        ));
    }
}

#[test]
fn a_playlist_delete_is_a_rest_delete_of_the_playlist_urn() {
    let Ok(request) = build(&claimed("playlist_delete", "soundcloud:playlists:5", None)) else {
        panic!("a playlist delete must build");
    };

    assert_eq!(request.method().as_str(), "DELETE");
    assert_eq!(
        path(&request).as_deref(),
        Some("/playlists/soundcloud:playlists:5")
    );
    assert!(build(&claimed("playlist_delete", "5", None)).is_err());
}

#[test]
fn a_create_with_fields_the_mobile_shape_does_not_send_stays_on_apiv1() {
    let plain = claimed(
        "playlist_create",
        "new:1",
        Some(json!({"playlist": {"title": "x", "sharing": "public", "tracks": [], "genre": null}})),
    );
    let described = claimed(
        "playlist_create",
        "new:1",
        Some(json!({"playlist": {"title": "x", "description": "d"}})),
    );

    assert!(carries_only_sent_fields(&plain));
    assert!(!carries_only_sent_fields(&described));
    assert!(carries_only_sent_fields(&claimed(
        "playlist_delete",
        "soundcloud:playlists:5",
        None
    )));
}

#[test]
fn a_mobile_create_reply_becomes_what_the_apiv1_reply_gives_the_local_playlist() {
    let sent = json!({"playlist": {"title": "Mix", "sharing": "private", "tracks": ["5", "6"]}});

    let by_urn = mapped(sent.clone(), json!({"urn": "soundcloud:playlists:77"}));
    let by_id = mapped(
        sent.clone(),
        json!({"playlist": {"id": 78, "title": "Mix"}}),
    );
    let listing_tracks = mapped(
        sent.clone(),
        json!({"urn": "soundcloud:playlists:79", "track_urns": ["soundcloud:tracks:9"]}),
    );
    let apiv1_like = mapped(
        sent.clone(),
        json!({
            "urn": "soundcloud:playlists:80", "title": "Mix", "sharing": "public",
            "user": {"id": 3, "urn": "soundcloud:users:3"}, "tracks": [{"id": 1}]
        }),
    );

    let by_urn = by_urn.unwrap_or_default();
    assert_eq!(by_urn["urn"], "soundcloud:playlists:77");
    assert_eq!(by_urn["title"], "Mix");
    assert_eq!(by_urn["sharing"], "private");
    assert_eq!(by_urn["user"]["urn"], "soundcloud:users:17");
    assert_eq!(
        by_urn["tracks"],
        json!([{"urn": "soundcloud:tracks:5"}, {"urn": "soundcloud:tracks:6"}])
    );
    let by_id = by_id.unwrap_or_default();
    assert_eq!(by_id["urn"], "soundcloud:playlists:78");
    let listing_tracks = listing_tracks.unwrap_or_default();
    assert_eq!(
        listing_tracks["tracks"],
        json!([{"urn": "soundcloud:tracks:9"}])
    );
    let apiv1_like = apiv1_like.unwrap_or_default();
    assert_eq!(apiv1_like["sharing"], "public");
    assert_eq!(apiv1_like["user"]["id"], 3);
    assert_eq!(apiv1_like["tracks"], json!([{"id": 1}]));
}

#[test]
fn a_create_reply_that_names_no_playlist_is_not_mapped() {
    let sent = json!({"playlist": {"title": "Mix"}});

    for reply in [
        json!({}),
        json!(null),
        json!({"playlist": {"title": "Mix"}}),
    ] {
        assert!(matches!(
            mapped(sent.clone(), reply),
            Err(ActionError::InvalidRemoteResult(_))
        ));
    }
}

#[test]
fn only_a_create_reply_is_reshaped() {
    let mutation = claimed("playlist_delete", "soundcloud:playlists:5", None);

    let kept = shape(&mutation, Remote::Written(json!({"ok": true})));

    assert!(matches!(kept, Ok(Remote::Written(value)) if value == json!({"ok": true})));
    assert!(matches!(
        shape(&mutation, Remote::TargetMissing),
        Ok(Remote::TargetMissing)
    ));
}
