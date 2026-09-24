use std::collections::HashMap;

use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicCollection {
    TrackLikes,
    PlaylistLikes,
    Playlists,
    Followings,
    OwnedTracks,
}

impl PublicCollection {
    pub fn lua_kind(self) -> &'static str {
        match self {
            Self::TrackLikes => "track_likes",
            Self::PlaylistLikes => "playlist_likes",
            Self::Playlists => "playlists",
            Self::Followings => "followings",
            Self::OwnedTracks => "tracks",
        }
    }

    pub fn path_segment(self) -> &'static str {
        match self {
            Self::TrackLikes => "track_likes",
            Self::PlaylistLikes => "playlist_likes",
            Self::Playlists => "playlists",
            Self::Followings => "followings",
            Self::OwnedTracks => "tracks",
        }
    }

    pub fn unwrap_field(self) -> Option<&'static str> {
        match self {
            Self::TrackLikes => Some("track"),
            Self::PlaylistLikes => Some("playlist"),
            Self::Playlists | Self::Followings | Self::OwnedTracks => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchType {
    Tracks,
    Users,
    PlaylistsWithoutAlbums,
}

impl SearchType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tracks => "tracks",
            Self::Users => "users",
            Self::PlaylistsWithoutAlbums => "playlists_without_albums",
        }
    }
}

pub fn unwrap_collection_items(items: &[Value], coll: PublicCollection) -> Vec<Value> {
    let unwrap = coll.unwrap_field();
    items
        .iter()
        .filter_map(|item| {
            let mut entity = match unwrap {
                Some(field) => item.get(field).cloned()?,
                None => item.clone(),
            };
            entity.get("id")?;
            normalize_v2_to_v1(&mut entity);
            Some(entity)
        })
        .collect()
}

pub fn collect_playlist_track_ids(playlist: &Value) -> (Vec<String>, HashMap<String, Value>) {
    let mut ids = Vec::new();
    let mut embedded = HashMap::new();
    if let Some(arr) = playlist.get("tracks").and_then(Value::as_array) {
        for t in arr {
            let Some(key) = t.get("id").and_then(id_to_string) else {
                continue;
            };
            ids.push(key.clone());
            if t.get("title").is_some() {
                embedded.insert(key, t.clone());
            }
        }
    }
    (ids, embedded)
}

pub fn reassemble_playlist_tracks(
    ordered_ids: &[String],
    embedded_full: &HashMap<String, Value>,
    hydrated: &HashMap<String, Value>,
) -> Vec<Value> {
    ordered_ids
        .iter()
        .filter_map(|id| embedded_full.get(id).or_else(|| hydrated.get(id)).cloned())
        .map(|mut t| {
            normalize_v2_to_v1(&mut t);
            t
        })
        .collect()
}

pub fn index_tracks_by_id(tracks: &[Value]) -> HashMap<String, Value> {
    tracks
        .iter()
        .filter_map(|t| Some((t.get("id").and_then(id_to_string)?, t.clone())))
        .collect()
}

fn id_to_string(v: &Value) -> Option<String> {
    match v {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

pub fn normalize_v2_to_v1(value: &mut Value) {
    match value {
        Value::Object(obj) => {
            normalize_object(obj);
            for (_, v) in obj.iter_mut() {
                normalize_v2_to_v1(v);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                normalize_v2_to_v1(v);
            }
        }
        _ => {}
    }
}

fn normalize_object(obj: &mut Map<String, Value>) {
    if !obj.contains_key("favoritings_count")
        && let Some(v) = obj.get("likes_count").cloned()
    {
        obj.insert("favoritings_count".to_string(), v);
    }
    if !matches!(obj.get("urn"), Some(Value::String(_)))
        && let Some(urn) = synth_urn(obj)
    {
        obj.insert("urn".to_string(), Value::String(urn));
    }
}

fn synth_urn(obj: &Map<String, Value>) -> Option<String> {
    let kind = obj.get("kind").and_then(|v| v.as_str())?;
    let segment = match kind {
        "track" => "tracks",
        "playlist" => "playlists",
        "user" => "users",
        "system-playlist" => "system-playlists",
        _ => return None,
    };
    let id = obj.get("id").and_then(id_to_string)?;
    Some(format!("soundcloud:{segment}:{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TRACK_LIKES_PAGE: &str = r#"{
        "collection": [
            {"created_at": "2019-10-06T06:37:19Z", "kind": "like",
             "track": {"id": 588195402, "kind": "track", "title": "A", "likes_count": 7}},
            {"created_at": "2019-10-05T06:37:19Z", "kind": "like",
             "track": {"id": 100, "kind": "track", "title": "B", "likes_count": 3,
                       "urn": "soundcloud:tracks:100"}}
        ],
        "next_href": "https://api-v2.soundcloud.com/users/183/track_likes?offset=x&limit=2"
    }"#;

    const PLAYLIST_LIKES_PAGE: &str = r#"{
        "collection": [
            {"created_at": "2020-01-01T00:00:00Z", "kind": "playlist-like",
             "playlist": {"id": 7, "kind": "playlist", "title": "Mix", "likes_count": 9}}
        ]
    }"#;

    #[test]
    fn unwrap_track_likes_yields_bare_normalized_tracks() {
        let page: Value = serde_json::from_str(TRACK_LIKES_PAGE).unwrap();
        let items = page["collection"].as_array().unwrap();
        let out = unwrap_collection_items(items, PublicCollection::TrackLikes);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["id"], 588195402);
        assert_eq!(out[0]["title"], "A");
        assert_eq!(out[0]["favoritings_count"], 7);
        assert_eq!(out[0]["urn"], "soundcloud:tracks:588195402");
        assert_eq!(out[1]["urn"], "soundcloud:tracks:100");
    }

    #[test]
    fn unwrap_playlist_likes_unwraps_playlist_field() {
        let page: Value = serde_json::from_str(PLAYLIST_LIKES_PAGE).unwrap();
        let items = page["collection"].as_array().unwrap();
        let out = unwrap_collection_items(items, PublicCollection::PlaylistLikes);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["id"], 7);
        assert_eq!(out[0]["urn"], "soundcloud:playlists:7");
        assert_eq!(out[0]["favoritings_count"], 9);
    }

    #[test]
    fn bare_collection_is_not_unwrapped() {
        let items = vec![json!({"id": 1, "kind": "user", "username": "x"})];
        let out = unwrap_collection_items(&items, PublicCollection::Followings);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["urn"], "soundcloud:users:1");
    }

    #[test]
    fn malformed_items_dropped() {
        let items = vec![
            json!({"created_at": "t", "kind": "like"}),
            json!({"kind": "like", "track": {"kind": "track"}}),
        ];
        let out = unwrap_collection_items(&items, PublicCollection::TrackLikes);
        assert!(out.is_empty());
    }

    #[test]
    fn playlist_hydration_preserves_order_and_drops_missing() {
        let playlist = json!({
            "id": 18, "kind": "playlist",
            "tracks": [
                {"id": 290, "kind": "track", "title": "City Ports", "likes_count": 1},
                {"id": 293, "kind": "track"},
                {"id": 999, "kind": "track"}
            ]
        });
        let (ids, embedded) = collect_playlist_track_ids(&playlist);
        assert_eq!(ids, vec!["290", "293", "999"]);
        assert_eq!(embedded.len(), 1);
        assert!(embedded.contains_key("290"));

        let hydrated =
            index_tracks_by_id(&[json!({"id": 293, "kind": "track", "title": "Flickermood"})]);
        let tracks = reassemble_playlist_tracks(&ids, &embedded, &hydrated);
        let got: Vec<i64> = tracks.iter().map(|t| t["id"].as_i64().unwrap()).collect();
        assert_eq!(got, vec![290, 293]);
        assert_eq!(tracks[0]["title"], "City Ports");
        assert_eq!(tracks[0]["favoritings_count"], 1);
    }

    #[test]
    fn collect_handles_string_ids() {
        let playlist = json!({"id": 1, "tracks": [{"id": "abc", "title": "t"}]});
        let (ids, embedded) = collect_playlist_track_ids(&playlist);
        assert_eq!(ids, vec!["abc"]);
        assert!(embedded.contains_key("abc"));
    }
}
