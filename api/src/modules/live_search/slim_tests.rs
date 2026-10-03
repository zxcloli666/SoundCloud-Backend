use serde_json::{Value, json};

use super::query::LiveKind;
use super::slim::{playlist, slim, track, user};

fn v2_track(id: i64, policy: &str) -> Value {
    let mut raw = json!({
        "id": id,
        "kind": "track",
        "title": "Lucid Dreams",
        "duration": 239836,
        "full_duration": 239836,
        "artwork_url": "https://i1.sndcdn.com/a.jpg",
        "permalink_url": "https://soundcloud.com/juicewrld/lucid-dreams",
        "sharing": "public",
        "policy": policy,
        "monetization_model": "AD_SUPPORTED",
        "playback_count": 1_000_000,
        "likes_count": 5000,
        "description": "a very long description ".repeat(40),
        "media": {"transcodings": [{"url": "https://api-v2.soundcloud.com/media/x"}]},
        "station_urn": "soundcloud:system-playlists:track-stations:1",
        "publisher_metadata": {
            "id": 9,
            "artist": "Juice WRLD",
            "isrc": "USUG11800927",
            "contains_music": true
        },
        "user": {
            "id": 17,
            "kind": "user",
            "username": "Juice WRLD",
            "avatar_url": "https://i1.sndcdn.com/u.jpg",
            "permalink_url": "https://soundcloud.com/juicewrld",
            "verified": true,
            "followers_count": 900,
            "description": "bio"
        }
    });
    sc_transport::normalize_v2_to_v1(&mut raw);
    raw
}

#[test]
fn a_track_keeps_only_the_whitelist_and_gains_its_artist() {
    let kept = track(&v2_track(42, "ALLOW")).expect("an allowed track is kept");

    assert_eq!(kept["urn"], "soundcloud:tracks:42");
    assert_eq!(kept["title"], "Lucid Dreams");
    assert_eq!(kept["metadata_artist"], "Juice WRLD");
    assert_eq!(
        kept["publisher_metadata"],
        json!({"artist": "Juice WRLD", "isrc": "USUG11800927"})
    );
    assert_eq!(kept["access"], "playable");
    assert_eq!(
        kept["user"],
        json!({
            "id": 17,
            "urn": "soundcloud:users:17",
            "kind": "user",
            "username": "Juice WRLD",
            "avatar_url": "https://i1.sndcdn.com/u.jpg",
            "permalink_url": "https://soundcloud.com/juicewrld",
            "verified": true
        })
    );
    for dropped in [
        "description",
        "media",
        "station_urn",
        "policy",
        "monetization_model",
        "favoritings_count",
    ] {
        assert!(kept.get(dropped).is_none(), "{dropped} must not be stored");
    }
    assert!(
        kept.to_string().len() < 1024,
        "a slim track must stay well under a kilobyte, it is {} bytes",
        kept.to_string().len()
    );
}

#[test]
fn the_policy_becomes_access_and_a_blocked_track_is_dropped() {
    assert_eq!(
        track(&v2_track(1, "MONETIZE")).unwrap()["access"],
        "playable"
    );
    assert_eq!(track(&v2_track(1, "SNIP")).unwrap()["access"], "preview");
    assert_eq!(track(&v2_track(1, "BLOCK")), None);

    let mut v1 = v2_track(1, "ALLOW");
    let object = v1.as_object_mut().unwrap();
    object.remove("policy");
    object.insert("access".into(), json!("blocked"));
    assert_eq!(track(&v1), None, "a v1 blocked track is dropped too");
}

#[test]
fn an_item_without_identity_or_of_another_kind_is_dropped() {
    let mut nameless = v2_track(1, "ALLOW");
    nameless["title"] = json!("  ");
    assert_eq!(track(&nameless), None);

    let mut anonymous = v2_track(1, "ALLOW");
    anonymous.as_object_mut().unwrap().remove("urn");
    assert_eq!(track(&anonymous), None);

    let playlist_shaped =
        json!({"id": 5, "kind": "playlist", "urn": "soundcloud:playlists:5", "title": "x"});
    assert_eq!(track(&playlist_shaped), None);
    assert_eq!(user(&playlist_shaped), None);
    assert!(playlist(&playlist_shaped).is_some());
}

#[test]
fn a_page_is_deduplicated_and_hands_over_its_uploaders() {
    let raw = vec![
        v2_track(1, "ALLOW"),
        v2_track(1, "ALLOW"),
        v2_track(2, "BLOCK"),
        v2_track(3, "SNIP"),
    ];
    let slimmed = slim(LiveKind::Tracks, &raw);

    let urns: Vec<&str> = slimmed
        .items
        .iter()
        .filter_map(|item| item["urn"].as_str())
        .collect();
    assert_eq!(urns, ["soundcloud:tracks:1", "soundcloud:tracks:3"]);
    assert_eq!(slimmed.users.len(), 1, "one uploader is stashed once");
    assert_eq!(slimmed.users[0]["urn"], "soundcloud:users:17");
    assert_eq!(slimmed.users[0]["followers_count"], 900);
    assert!(slimmed.users[0].get("description").is_none());
}

#[test]
fn users_and_playlists_get_short_whitelists() {
    let mut raw_user = json!({
        "id": 17, "kind": "user", "username": "Juice WRLD", "followers_count": 900,
        "track_count": 120, "description": "bio", "visuals": {"visuals": []}
    });
    sc_transport::normalize_v2_to_v1(&mut raw_user);
    let kept = user(&raw_user).expect("a named user is kept");
    assert_eq!(kept["urn"], "soundcloud:users:17");
    assert_eq!(kept["track_count"], 120);
    assert!(kept.get("visuals").is_none());
    assert!(kept.get("description").is_none());

    let mut raw_playlist = json!({
        "id": 5, "kind": "playlist", "title": "Mix", "track_count": 12, "set_type": "",
        "tracks": [{"id": 1}], "user": {"id": 17, "kind": "user", "username": "dj"}
    });
    sc_transport::normalize_v2_to_v1(&mut raw_playlist);
    let kept = playlist(&raw_playlist).expect("a titled playlist is kept");
    assert_eq!(kept["urn"], "soundcloud:playlists:5");
    assert!(kept.get("tracks").is_none(), "membership never rides along");
    assert_eq!(kept["user"]["urn"], "soundcloud:users:17");
    assert_eq!(slim(LiveKind::Playlists, &[raw_playlist]).users.len(), 1);
}
