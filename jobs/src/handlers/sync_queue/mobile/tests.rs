use bytes::Bytes;
use chrono::Utc;
use sc_mobile::{Kind, MobileReply};
use serde_json::{Value, json};
use uuid::Uuid;

use super::super::actions::ActionError;
use super::super::client::SoundCloudError;
use super::super::model::ClaimedMutation;
use super::{Route, build, judge, route};

fn claimed(action: &str, target: &str, payload: Option<Value>) -> ClaimedMutation {
    ClaimedMutation {
        id: Uuid::new_v4(),
        user_id: "17".to_owned(),
        action_type: action.to_owned(),
        target_urn: target.to_owned(),
        payload,
        retry_count: 0,
        generation: 1,
        lease_id: Uuid::new_v4(),
        lease_generation: 1,
        remote_attempted_generation: None,
        remote_completed_generation: None,
        remote_result: None,
        created_at: Utc::now(),
        star: false,
        desired: None,
    }
}

fn reply(status: u16, body: &'static str) -> MobileReply {
    MobileReply {
        status,
        plan: None,
        location: None,
        body: Bytes::from_static(body.as_bytes()),
    }
}

#[test]
fn only_reactions_comment_delete_and_follows_leave_apiv1() {
    for action in [
        "track_reaction_set",
        "track_reaction_remove",
        "comment_delete",
    ] {
        assert_eq!(route(action), Route::Mobile, "{action}");
    }
    for action in ["follow_user", "unfollow_user"] {
        assert_eq!(route(action), Route::MobileThenApiv1, "{action}");
    }
    for action in [
        "like_track",
        "unlike_track",
        "like_playlist",
        "unlike_playlist",
        "comment",
        "playlist_create",
        "playlist_delete",
        "playlist_update",
        "playlist_membership",
        "track_update",
        "track_delete",
        "repost_track",
        "comment_like",
    ] {
        assert_eq!(route(action), Route::Apiv1, "{action}");
    }
}

#[test]
fn a_reaction_builds_the_graphql_mutation_with_the_moment_and_the_default_value() {
    let set = build(&claimed(
        "track_reaction_set",
        "soundcloud:tracks:42",
        Some(json!({"at_second": 5})),
    ));
    let remove = build(&claimed(
        "track_reaction_remove",
        "soundcloud:tracks:42",
        Some(json!({"at_second": 5, "value_urn": "sc:interactiontypevalue:1f525"})),
    ));

    let (Ok(set), Ok(remove)) = (set, remove) else {
        panic!("a reaction must build");
    };
    assert_eq!(set.kind(), Kind::Mutation);
    assert_eq!(remove.kind(), Kind::Mutation);
    let input = |request: &sc_mobile::MobileRequest| -> Option<Value> {
        let body: Value = serde_json::from_slice(request.body()?).ok()?;
        body.pointer("/variables/input").cloned()
    };
    let expected = json!({
        "targetUrn": "ts:5",
        "parentUrn": "soundcloud:tracks:42",
        "interactionTypeUrn": "sc:interactiontype:trackreaction",
        "interactionTypeValueUrn": "sc:interactiontypevalue:1f525",
    });
    assert_eq!(input(&set), Some(expected.clone()));
    assert_eq!(input(&remove), Some(expected));
    let query = |request: &sc_mobile::MobileRequest| -> Option<String> {
        let body: Value = serde_json::from_slice(request.body()?).ok()?;
        body.get("query")?.as_str().map(str::to_owned)
    };
    assert!(query(&set).is_some_and(|query| query.contains("upsertInteraction")));
    assert!(query(&remove).is_some_and(|query| query.contains("removeInteraction")));
}

#[test]
fn a_comment_delete_is_a_rest_delete_of_the_comment_urn() {
    let Ok(request) = build(&claimed("comment_delete", "soundcloud:comments:5", None)) else {
        panic!("a comment delete must build");
    };

    assert_eq!(request.method().as_str(), "DELETE");
    assert_eq!(request.kind(), Kind::Write);
    assert_eq!(request.body(), None);
    assert_eq!(
        request
            .url()
            .ok()
            .map(|url| url.path().to_owned())
            .as_deref(),
        Some("/comments/soundcloud:comments:5")
    );
}

#[test]
fn follows_are_rest_writes_on_the_user_urn() {
    let Ok(follow) = build(&claimed("follow_user", "soundcloud:users:9", None)) else {
        panic!("a follow must build");
    };
    let Ok(unfollow) = build(&claimed("unfollow_user", "soundcloud:users:9", None)) else {
        panic!("an unfollow must build");
    };

    assert_eq!(follow.method().as_str(), "POST");
    assert_eq!(unfollow.method().as_str(), "DELETE");
    for request in [&follow, &unfollow] {
        assert_eq!(
            request
                .url()
                .ok()
                .map(|url| url.path().to_owned())
                .as_deref(),
            Some("/follows/users/soundcloud:users:9")
        );
    }
}

#[test]
fn a_malformed_mutation_is_refused_before_any_request_is_built() {
    let bad = [
        claimed("track_reaction_set", "soundcloud:tracks:42", None),
        claimed(
            "track_reaction_set",
            "soundcloud:tracks:42",
            Some(json!({})),
        ),
        claimed(
            "track_reaction_set",
            "soundcloud:tracks:42",
            Some(json!({"at_second": -1})),
        ),
        claimed(
            "track_reaction_set",
            "soundcloud:tracks:42",
            Some(json!({"at_second": 1, "value_urn": "other"})),
        ),
        claimed(
            "track_reaction_set",
            "soundcloud:users:42",
            Some(json!({"at_second": 1})),
        ),
        claimed("comment_delete", "42", None),
    ];

    for mutation in &bad {
        assert!(
            matches!(build(mutation), Err(ActionError::InvalidPayload(_))),
            "{}",
            mutation.action_type
        );
    }
    assert!(matches!(
        build(&claimed("like_track", "soundcloud:tracks:42", None)),
        Err(ActionError::UnknownAction(_))
    ));
}

#[test]
fn a_reply_is_a_write_only_when_it_is_a_success_without_graph_errors() {
    let Ok(mutation) = build(&claimed(
        "track_reaction_set",
        "soundcloud:tracks:42",
        Some(json!({"at_second": 5})),
    )) else {
        panic!("a reaction must build");
    };
    let Ok(delete) = build(&claimed("comment_delete", "soundcloud:comments:5", None)) else {
        panic!("a delete must build");
    };

    assert!(
        judge(
            &mutation,
            &reply(
                200,
                r#"{"data":{"upsertInteraction":{"targetUrn":"ts:5"}}}"#
            )
        )
        .is_ok()
    );
    assert!(matches!(
        judge(&mutation, &reply(200, r#"{"errors":[{"message":"x"}]}"#)),
        Err(SoundCloudError::Mobile(_))
    ));
    assert!(judge(&delete, &reply(204, "")).is_ok());
    assert!(matches!(
        judge(&delete, &reply(404, "")),
        Err(SoundCloudError::Api { status, .. }) if status.as_u16() == 404
    ));
    assert!(matches!(
        judge(&delete, &reply(429, "")),
        Err(error) if error.is_rate_limited()
    ));
}
