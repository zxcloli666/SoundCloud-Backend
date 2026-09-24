use backend_contracts::{CatalogCollection, CatalogCollectionPayload};
use chrono::{DateTime, Utc};
use serde_json::Value;
use wreq::Url;

use crate::queue::{JobError, JobResult};

pub(super) const PAGE_SIZE: usize = 100;

pub(super) struct Page {
    pub items: Vec<Value>,
    pub next: Option<String>,
    pub liked_at: Vec<(String, DateTime<Utc>)>,
}

fn like_time(
    payload: &CatalogCollectionPayload,
    item: &Value,
    apiv2: bool,
) -> Option<DateTime<Utc>> {
    if !apiv2 || payload.collection != CatalogCollection::LikedTracks {
        return None;
    }
    catalog_ingest::parse_dt(item.get("created_at")).filter(|liked_at| *liked_at <= Utc::now())
}

pub(super) fn parse(
    payload: &CatalogCollectionPayload,
    value: Value,
    apiv2: bool,
) -> JobResult<Page> {
    let collection = value
        .get("collection")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("collection array is missing"))?;
    if collection.len() > PAGE_SIZE {
        return Err(malformed("collection page is too large"));
    }
    let next = match value.get("next_href") {
        None | Some(Value::Null) => None,
        Some(Value::String(next)) if next.is_empty() => None,
        Some(Value::String(next)) => Some(cursor(payload, next, apiv2)?),
        Some(_) => return Err(malformed("collection cursor is malformed")),
    };
    if collection.is_empty() && next.is_some() {
        return Err(malformed("empty collection page has a continuation"));
    }
    if payload.collection.item() == backend_contracts::CollectionItem::Comment {
        let mut items = Vec::with_capacity(collection.len());
        for item in collection {
            items.push(comment(payload, item)?);
        }
        return Ok(Page {
            items,
            next,
            liked_at: Vec::new(),
        });
    }
    let mut items = Vec::with_capacity(collection.len());
    let mut liked_at: Vec<(String, DateTime<Utc>)> = Vec::new();
    for item in collection {
        let mut entity = match payload.collection {
            CatalogCollection::LikedTracks if apiv2 => item.get("track"),
            CatalogCollection::LikedPlaylists if apiv2 => item.get("playlist"),
            _ => Some(item),
        }
        .cloned()
        .ok_or_else(|| malformed("collection entity is missing"))?;
        sc_transport::normalize_v2_to_v1(&mut entity);
        let id = entity
            .get("urn")
            .and_then(Value::as_str)
            .map(catalog_ingest::extract_sc_id)
            .map(str::to_owned)
            .or_else(|| {
                entity
                    .get("id")
                    .and_then(Value::as_i64)
                    .map(|id| id.to_string())
            })
            .filter(|id| {
                id.parse::<i64>()
                    .is_ok_and(|number| number > 0 && number.to_string() == *id)
            })
            .ok_or_else(|| malformed("collection entity identity is missing"))?;
        let urn = payload.collection.entity().urn(&id);
        if entity
            .get("urn")
            .and_then(Value::as_str)
            .is_some_and(|value| value != urn)
            || entity
                .get("id")
                .and_then(Value::as_i64)
                .is_some_and(|number| number.to_string() != id)
        {
            return Err(malformed("collection entity identity mismatch"));
        }
        let is_user = payload.collection.entity() == backend_contracts::CatalogEntity::User;
        let title = if is_user { "username" } else { "title" };
        if entity
            .get(title)
            .and_then(Value::as_str)
            .is_none_or(|title| title.trim().is_empty())
        {
            return Err(malformed("collection entity title is missing"));
        }
        if !is_user {
            match entity.get("sharing").and_then(Value::as_str) {
                Some("public") => {}
                Some("private") if payload.owner => {}
                _ => return Err(malformed("collection visibility is invalid")),
            }
        }
        if matches!(
            payload.collection,
            CatalogCollection::OwnedTracks | CatalogCollection::OwnedPlaylists
        ) && entity
            .pointer("/user/urn")
            .and_then(Value::as_str)
            .map(catalog_ingest::extract_sc_id)
            != Some(&payload.subject_id)
        {
            return Err(malformed("owned collection has an unrelated entity"));
        }
        if let Some(object) = entity.as_object_mut() {
            object.insert("urn".into(), Value::String(urn));
        }
        if let Some(time) = like_time(payload, item, apiv2)
            && !liked_at.iter().any(|(known, _)| *known == id)
        {
            liked_at.push((id, time));
        }
        items.push(entity);
    }
    Ok(Page {
        items,
        next,
        liked_at,
    })
}

const MAX_COMMENT_BYTES: usize = 16 * 1024;

fn numeric_id(value: Option<&Value>) -> Option<String> {
    let id = match value? {
        Value::Number(number) => number.as_i64()?.to_string(),
        Value::String(text) => text.to_owned(),
        _ => return None,
    };
    id.parse::<i64>()
        .is_ok_and(|number| number > 0 && number.to_string() == id)
        .then_some(id)
}

fn identity(object: &Value) -> Option<String> {
    if let Some(id) = numeric_id(object.get("id")) {
        return Some(id);
    }
    let urn = object.get("urn").and_then(Value::as_str)?;
    numeric_id(Some(&Value::String(
        catalog_ingest::extract_sc_id(urn).to_owned(),
    )))
}

fn comment(payload: &CatalogCollectionPayload, item: &Value) -> JobResult<Value> {
    let mut comment = item.clone();
    sc_transport::normalize_v2_to_v1(&mut comment);
    let id = identity(&comment).ok_or_else(|| malformed("comment identity is missing"))?;
    let body = comment
        .get("body")
        .and_then(Value::as_str)
        .filter(|body| !body.trim().is_empty() && body.len() <= MAX_COMMENT_BYTES)
        .ok_or_else(|| malformed("comment body is missing or oversized"))?
        .to_owned();
    let user = comment
        .get("user")
        .filter(|user| user.is_object())
        .cloned()
        .ok_or_else(|| malformed("comment author is missing"))?;
    let author = identity(&user).ok_or_else(|| malformed("comment author identity is missing"))?;
    if user
        .get("username")
        .and_then(Value::as_str)
        .is_none_or(|username| username.trim().is_empty())
    {
        return Err(malformed("comment author name is missing"));
    }
    if numeric_id(comment.get("track_id")).is_some_and(|track| track != payload.subject_id) {
        return Err(malformed("comment belongs to another track"));
    }
    let position = match comment.get("timestamp") {
        None | Some(Value::Null) => None,
        Some(Value::Number(number)) => Some(
            number
                .as_i64()
                .filter(|value| *value >= 0)
                .ok_or_else(|| malformed("comment position is invalid"))?,
        ),
        Some(_) => return Err(malformed("comment position is invalid")),
    };
    let created_at = match comment.get("created_at") {
        None | Some(Value::Null) => None,
        Some(value @ Value::String(_)) => Some(
            catalog_ingest::parse_dt(Some(value))
                .ok_or_else(|| malformed("comment date is invalid"))?
                .to_rfc3339(),
        ),
        Some(_) => return Err(malformed("comment date is invalid")),
    };
    let mut user = user;
    if let Some(object) = user.as_object_mut() {
        object.insert(
            "urn".into(),
            Value::String(backend_contracts::CatalogEntity::User.urn(&author)),
        );
    }
    Ok(serde_json::json!({
        "id": id,
        "body": body,
        "timestamp": position,
        "created_at": created_at,
        "track_id": payload.subject_id,
        "user": user,
    }))
}

fn cursor(payload: &CatalogCollectionPayload, value: &str, apiv2: bool) -> JobResult<String> {
    if value.len() > 2048 {
        return Err(malformed("collection cursor is too large"));
    }
    let url = Url::parse(value).map_err(|_| malformed("collection cursor URL is invalid"))?;
    let host = if apiv2 {
        "api-v2.soundcloud.com"
    } else {
        "api.soundcloud.com"
    };
    let opaque_collection = !apiv2
        && url.path() == "/collection"
        && url
            .query_pairs()
            .any(|(key, value)| key == "cursor" && !value.is_empty());
    if url.scheme() != "https"
        || url.host_str() != Some(host)
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || (!opaque_collection && url.path() != payload.path(apiv2))
    {
        return Err(malformed(
            "collection cursor changed its resource or origin",
        ));
    }
    let mut pairs: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| {
            !matches!(
                key.as_ref(),
                "client_id" | "access_token" | "oauth_token" | "authorization"
            )
        })
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    pairs.sort();
    let mut normalized = Url::parse(&format!("https://{host}{}", url.path()))
        .map_err(|_| malformed("collection cursor path is invalid"))?;
    normalized.query_pairs_mut().extend_pairs(pairs);
    Ok(format!(
        "{}:{}",
        if apiv2 { "v2" } else { "v1" },
        &normalized[url::Position::BeforePath..]
    ))
}

pub(super) fn target(payload: &CatalogCollectionPayload, saved: &str) -> JobResult<(bool, String)> {
    let (version, path) = saved
        .split_once(':')
        .ok_or_else(|| malformed("saved cursor version is missing"))?;
    let apiv2 = match version {
        "v2" if !payload.owner && payload.collection.public_apiv2() => true,
        "v1" => false,
        _ => return Err(malformed("saved cursor version is invalid")),
    };
    let host = if apiv2 {
        "api-v2.soundcloud.com"
    } else {
        "api.soundcloud.com"
    };
    if cursor(payload, &format!("https://{host}{path}"), apiv2)? != saved {
        return Err(malformed("saved cursor is not canonical"));
    }
    Ok((apiv2, path.to_owned()))
}

pub(super) fn request_path(payload: &CatalogCollectionPayload, path: &str) -> JobResult<String> {
    let mut url = Url::parse(&format!("https://api.soundcloud.com{path}"))
        .map_err(|_| malformed("collection request path is invalid"))?;
    let pairs: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| key != "access" && key != "show_tracks" && key != "threaded")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    url.query_pairs_mut().extend_pairs(pairs);
    match payload.collection {
        CatalogCollection::LikedTracks | CatalogCollection::OwnedTracks => {
            url.query_pairs_mut()
                .append_pair("access", "playable,preview,blocked");
        }
        CatalogCollection::LikedPlaylists | CatalogCollection::OwnedPlaylists => {
            url.query_pairs_mut().append_pair("show_tracks", "false");
        }
        CatalogCollection::TrackComments => {
            url.query_pairs_mut().append_pair("threaded", "0");
        }
        CatalogCollection::Followings
        | CatalogCollection::Followers
        | CatalogCollection::TrackFavoriters
        | CatalogCollection::TrackReposters
        | CatalogCollection::PlaylistReposters => {}
    }
    Ok(url[url::Position::BeforePath..].to_owned())
}

fn malformed(message: &'static str) -> JobError {
    JobError::retryable(anyhow::anyhow!(message))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload() -> CatalogCollectionPayload {
        CatalogCollectionPayload {
            collection: CatalogCollection::LikedTracks,
            subject_id: "42".into(),
            owner: false,
        }
    }

    #[test]
    fn malformed_pages_cannot_certify_an_empty_collection() {
        for value in [
            json!({}),
            json!({"collection": {}}),
            json!({"collection": [{"track": null}]}),
            json!({"collection": [], "next_href": 2}),
        ] {
            assert!(parse(&payload(), value, true).is_err());
        }
        assert!(
            parse(
                &payload(),
                json!({"collection": [], "next_href": null}),
                true
            )
            .is_ok()
        );
    }

    #[test]
    fn cursor_is_scoped_and_contains_no_credentials() {
        let payload = payload();
        let saved = cursor(&payload, "https://api-v2.soundcloud.com/users/42/track_likes?offset=100&client_id=secret&access_token=secret", true).unwrap();
        assert_eq!(saved, "v2:/users/42/track_likes?offset=100");
        assert_eq!(
            target(&payload, &saved).unwrap(),
            (true, "/users/42/track_likes?offset=100".into())
        );
        for url in [
            "https://evil.example/users/42/track_likes?offset=100",
            "https://api-v2.soundcloud.com/users/43/track_likes?offset=100",
            "https://api-v2.soundcloud.com@evil.example/users/42/track_likes",
            "http://api-v2.soundcloud.com/users/42/track_likes",
        ] {
            assert!(cursor(&payload, url, true).is_err());
        }
    }

    #[test]
    fn likes_are_unwrapped_and_private_data_requires_the_owner_scope() {
        let value =
            json!({"collection": [{"track": {"id": 7, "title": "Track", "sharing": "public"}}]});
        let parsed = parse(&payload(), value, true).unwrap();
        assert_eq!(parsed.items[0]["urn"], "soundcloud:tracks:7");
        let value =
            json!({"collection": [{"track": {"id": 7, "title": "Track", "sharing": "private"}}]});
        assert!(parse(&payload(), value, true).is_err());
    }

    #[test]
    fn a_public_like_carries_the_time_of_the_like_and_not_of_the_track() {
        let value = json!({"collection": [
            {"created_at": "2019-10-06T06:37:19Z", "kind": "like",
             "track": {"id": 7, "title": "Track", "sharing": "public", "created_at": "2012-01-01T00:00:00Z"}},
            {"kind": "like", "track": {"id": 8, "title": "Other", "sharing": "public", "created_at": "2013-01-01T00:00:00Z"}},
            {"created_at": "not a date", "kind": "like", "track": {"id": 9, "title": "Third", "sharing": "public"}},
            {"created_at": "2999-01-01T00:00:00Z", "kind": "like", "track": {"id": 10, "title": "Future", "sharing": "public"}}
        ]});
        let parsed = parse(&payload(), value, true).unwrap();
        let expected = DateTime::parse_from_rfc3339("2019-10-06T06:37:19Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(parsed.liked_at, vec![("7".to_owned(), expected)]);
        assert_eq!(parsed.items.len(), 4);
    }

    #[test]
    fn owner_likes_and_other_collections_never_invent_a_like_time() {
        let mut payload = payload();
        payload.owner = true;
        let value = json!({"collection": [{"id": 7, "title": "Track", "sharing": "public", "created_at": "2012-01-01T00:00:00Z"}]});
        assert!(
            parse(&payload, value.clone(), false)
                .unwrap()
                .liked_at
                .is_empty()
        );
        payload.owner = false;
        assert!(parse(&payload, value, false).unwrap().liked_at.is_empty());
        payload.collection = CatalogCollection::LikedPlaylists;
        let value = json!({"collection": [{"created_at": "2019-10-06T06:37:19Z", "kind": "playlist-like",
            "playlist": {"id": 7, "title": "Mix", "sharing": "public"}}]});
        assert!(parse(&payload, value, true).unwrap().liked_at.is_empty());
    }

    #[test]
    fn owner_api_accepts_urn_identity_and_rejects_conflicting_ids() {
        let mut payload = payload();
        payload.owner = true;
        let value = json!({"collection": [{"urn": "soundcloud:tracks:7", "title": "Track", "sharing": "private"}]});
        assert!(parse(&payload, value, false).is_ok());
        for urn in [
            "soundcloud:users:7",
            "soundcloud:tracks:007",
            "soundcloud:tracks:0",
        ] {
            assert!(
                parse(
                    &payload,
                    json!({"collection": [{"urn": urn, "title": "Track", "sharing": "public"}]}),
                    false
                )
                .is_err()
            );
        }
        assert!(parse(&payload, json!({"collection": [{"urn": "soundcloud:tracks:7", "id": 8, "title": "Track", "sharing": "public"}]}), false).is_err());
    }

    #[test]
    fn owner_pagination_accepts_the_documented_opaque_collection_cursor() {
        let mut payload = payload();
        payload.owner = true;
        let saved = cursor(
            &payload,
            "https://api.soundcloud.com/collection?page_size=100&cursor=1234567",
            false,
        )
        .unwrap();
        assert_eq!(
            target(&payload, &saved).unwrap(),
            (false, "/collection?cursor=1234567&page_size=100".into())
        );
        assert!(
            cursor(
                &payload,
                "https://api.soundcloud.com/collection?page_size=100",
                false
            )
            .is_err()
        );
    }

    #[test]
    fn favoriters_stay_on_the_transport_that_serves_them() {
        let mut payload = payload();
        payload.collection = CatalogCollection::TrackFavoriters;
        payload.owner = false;
        assert!(!payload.collection.public_apiv2());
        let saved = cursor(
            &payload,
            "https://api.soundcloud.com/tracks/42/favoriters?offset=100",
            false,
        )
        .unwrap();
        assert_eq!(
            target(&payload, &saved).unwrap(),
            (false, "/tracks/42/favoriters?offset=100".into())
        );
        assert!(target(&payload, "v2:/tracks/42/favoriters?offset=100").is_err());
        payload.collection = CatalogCollection::TrackReposters;
        assert!(payload.collection.public_apiv2());
        let saved = cursor(
            &payload,
            "https://api-v2.soundcloud.com/tracks/42/reposters?offset=100",
            true,
        )
        .unwrap();
        assert_eq!(
            target(&payload, &saved).unwrap(),
            (true, "/tracks/42/reposters?offset=100".into())
        );
    }

    #[test]
    fn every_page_requests_the_full_access_range_and_metadata_only_playlists() {
        let mut payload = payload();
        let path = request_path(&payload, "/collection?cursor=opaque&access=playable").unwrap();
        assert_eq!(
            path,
            "/collection?cursor=opaque&access=playable%2Cpreview%2Cblocked"
        );
        payload.collection = CatalogCollection::OwnedPlaylists;
        let path = request_path(&payload, "/me/playlists?limit=100&show_tracks=true").unwrap();
        assert_eq!(path, "/me/playlists?limit=100&show_tracks=false");
    }
}
