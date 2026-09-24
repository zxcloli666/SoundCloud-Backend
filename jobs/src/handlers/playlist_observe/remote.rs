use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::time::timeout;
use wreq::Url;

use super::client::{PlaylistReadClient, PlaylistReadError};
use super::model::{HydratedTrack, PlaylistSnapshot};
use super::urn::PlaylistUrn;

const PAGE_SIZE: usize = 200;
const MAX_PAGES: usize = 100;
const MAX_TRACKS: usize = PAGE_SIZE * MAX_PAGES;
const MAX_OBSERVATION_BYTES: usize = 16 * 1024 * 1024;
const OBSERVATION_DEADLINE: Duration = Duration::from_secs(60);

pub struct PlaylistReader {
    client: PlaylistReadClient,
}

#[derive(Debug, thiserror::Error)]
pub enum PlaylistObserveError {
    #[error(transparent)]
    Read(#[from] PlaylistReadError),

    #[error("SoundCloud playlist observation exceeded its deadline")]
    Deadline,

    #[error("SoundCloud playlist response is invalid: {0}")]
    Invalid(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RemoteState {
    playlist_id: String,
    owner_id: String,
    track_count: usize,
    last_modified: DateTime<Utc>,
}

#[derive(Debug)]
struct TrackPage {
    track_ids: Vec<String>,
    hydrated_tracks: Vec<HydratedTrack>,
    next_href: Option<String>,
}

struct ObservedMembership {
    track_ids: Vec<String>,
    hydrated_tracks: Vec<HydratedTrack>,
}

#[derive(Default)]
struct BodyBudget {
    used: usize,
}

impl PlaylistReader {
    pub fn new(client: PlaylistReadClient) -> Self {
        Self { client }
    }

    pub async fn observe(
        &self,
        urn: &PlaylistUrn,
        access_token: &str,
    ) -> Result<PlaylistSnapshot, PlaylistObserveError> {
        timeout(
            OBSERVATION_DEADLINE,
            self.observe_before_deadline(urn, access_token),
        )
        .await
        .map_err(|_| PlaylistObserveError::Deadline)?
    }

    async fn observe_before_deadline(
        &self,
        urn: &PlaylistUrn,
        access_token: &str,
    ) -> Result<PlaylistSnapshot, PlaylistObserveError> {
        let mut budget = BodyBudget::default();
        let before = self.load_state(urn, access_token, &mut budget).await?;
        let membership = self
            .load_membership(urn, before.track_count, access_token, &mut budget)
            .await?;
        let after = self.load_state(urn, access_token, &mut budget).await?;
        if before != after {
            return Err(PlaylistObserveError::Invalid(
                "playlist changed while it was being observed",
            ));
        }
        Ok(PlaylistSnapshot {
            playlist_id: before.playlist_id,
            owner_id: before.owner_id,
            track_count: i32::try_from(before.track_count)
                .map_err(|_| PlaylistObserveError::Invalid("track count is too large"))?,
            track_ids: membership.track_ids,
            hydrated_tracks: membership.hydrated_tracks,
            remote_last_modified: before.last_modified,
            observed_at: Utc::now(),
        })
    }

    async fn load_state(
        &self,
        urn: &PlaylistUrn,
        access_token: &str,
        budget: &mut BodyBudget,
    ) -> Result<RemoteState, PlaylistObserveError> {
        let response = self
            .client
            .get_path(&format!("/playlists/{}", urn.id()), access_token)
            .await?;
        budget.charge(response.body_bytes)?;
        RemoteState::parse(&response.value, urn)
    }

    async fn load_membership(
        &self,
        urn: &PlaylistUrn,
        expected_count: usize,
        access_token: &str,
        budget: &mut BodyBudget,
    ) -> Result<ObservedMembership, PlaylistObserveError> {
        let expected_path = format!("/playlists/{}/tracks", urn.id());
        let mut next = Some(format!(
            "{expected_path}?limit={PAGE_SIZE}&linked_partitioning=true"
        ));
        let mut visited = HashSet::new();
        let mut track_ids = Vec::with_capacity(expected_count);
        let mut hydrated_tracks = Vec::with_capacity(expected_count);
        let mut seen_ids = HashSet::with_capacity(expected_count);
        for _ in 0..MAX_PAGES {
            let Some(target) = next.take() else {
                break;
            };
            if !visited.insert(target.clone()) {
                return Err(PlaylistObserveError::Invalid(
                    "playlist pagination repeated a page",
                ));
            }
            let response = if target.starts_with('/') {
                self.client.get_path(&target, access_token).await?
            } else {
                validate_next_target(&target, &expected_path)?;
                self.client.get_next(&target, access_token).await?
            };
            budget.charge(response.body_bytes)?;
            let page = TrackPage::parse(&response.value)?;
            if page.track_ids.is_empty() && page.next_href.is_some() {
                return Err(PlaylistObserveError::Invalid(
                    "playlist pagination did not advance",
                ));
            }
            for track_id in page.track_ids {
                if !seen_ids.insert(track_id.clone()) {
                    return Err(PlaylistObserveError::Invalid(
                        "playlist contains duplicate track IDs",
                    ));
                }
                track_ids.push(track_id);
            }
            hydrated_tracks.extend(page.hydrated_tracks);
            if track_ids.len() > expected_count || track_ids.len() > MAX_TRACKS {
                return Err(PlaylistObserveError::Invalid(
                    "playlist returned more tracks than declared",
                ));
            }
            if page.next_href.is_some() && track_ids.len() == expected_count {
                return Err(PlaylistObserveError::Invalid(
                    "playlist pagination continues past the declared track count",
                ));
            }
            next = page.next_href;
        }
        if next.is_some() {
            return Err(PlaylistObserveError::Invalid(
                "playlist pagination exceeded the page limit",
            ));
        }
        if track_ids.len() != expected_count {
            return Err(PlaylistObserveError::Invalid(
                "playlist pagination ended before the declared track count",
            ));
        }
        Ok(ObservedMembership {
            track_ids,
            hydrated_tracks,
        })
    }
}

impl PlaylistObserveError {
    pub fn read(&self) -> Option<&PlaylistReadError> {
        match self {
            Self::Read(error) => Some(error),
            Self::Deadline | Self::Invalid(_) => None,
        }
    }
}

impl BodyBudget {
    fn charge(&mut self, bytes: usize) -> Result<(), PlaylistObserveError> {
        self.used = self.used.saturating_add(bytes);
        if self.used > MAX_OBSERVATION_BYTES {
            return Err(PlaylistObserveError::Invalid(
                "playlist observation exceeded the aggregate body limit",
            ));
        }
        Ok(())
    }
}

impl RemoteState {
    fn parse(value: &Value, urn: &PlaylistUrn) -> Result<Self, PlaylistObserveError> {
        let playlist_id = numeric_id(value.get("id"))
            .or_else(|| urn_id(value.get("urn"), "soundcloud:playlists:"))
            .ok_or(PlaylistObserveError::Invalid("playlist ID is missing"))?;
        if playlist_id != urn.id() {
            return Err(PlaylistObserveError::Invalid("playlist ID does not match"));
        }
        let owner = value
            .get("user")
            .ok_or(PlaylistObserveError::Invalid("playlist owner is missing"))?;
        let owner_id = numeric_id(owner.get("id"))
            .or_else(|| urn_id(owner.get("urn"), "soundcloud:users:"))
            .ok_or(PlaylistObserveError::Invalid(
                "playlist owner ID is missing",
            ))?;
        let track_count = value
            .get("track_count")
            .and_then(Value::as_u64)
            .and_then(|count| usize::try_from(count).ok())
            .filter(|count| *count <= MAX_TRACKS)
            .ok_or(PlaylistObserveError::Invalid(
                "playlist track count is invalid",
            ))?;
        let last_modified = value
            .get("last_modified")
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
            .ok_or(PlaylistObserveError::Invalid(
                "playlist last-modified timestamp is invalid",
            ))?;
        Ok(Self {
            playlist_id,
            owner_id,
            track_count,
            last_modified,
        })
    }
}

impl TrackPage {
    fn parse(value: &Value) -> Result<Self, PlaylistObserveError> {
        let collection = value.get("collection").and_then(Value::as_array).ok_or(
            PlaylistObserveError::Invalid("playlist track page has no collection"),
        )?;
        if collection.len() > PAGE_SIZE {
            return Err(PlaylistObserveError::Invalid(
                "playlist track page exceeded its item limit",
            ));
        }
        let mut track_ids = Vec::with_capacity(collection.len());
        let mut hydrated_tracks = Vec::with_capacity(collection.len());
        for track in collection {
            let track_id = track_id(track)?;
            if let Some(hydrated) = hydrate_track(track, &track_id) {
                hydrated_tracks.push(hydrated);
            }
            track_ids.push(track_id);
        }
        let next_href = match value.get("next_href") {
            None | Some(Value::Null) => None,
            Some(Value::String(value)) if !value.trim().is_empty() => Some(value.to_owned()),
            Some(_) => {
                return Err(PlaylistObserveError::Invalid(
                    "playlist track page has an invalid next link",
                ));
            }
        };
        Ok(Self {
            track_ids,
            hydrated_tracks,
            next_href,
        })
    }
}

fn hydrate_track(value: &Value, track_id: &str) -> Option<HydratedTrack> {
    let title = non_empty_string(value, "title")?;
    let urn = value
        .get("urn")
        .and_then(Value::as_str)
        .filter(|urn| urn.strip_prefix("soundcloud:tracks:") == Some(track_id))
        .map(str::to_owned)
        .unwrap_or_else(|| format!("soundcloud:tracks:{track_id}"));
    let full_duration = value.get("full_duration").and_then(Value::as_i64);
    let duration = full_duration
        .or_else(|| value.get("duration").and_then(Value::as_i64))
        .and_then(|duration| i32::try_from(duration.max(0)).ok())
        .unwrap_or_default();
    let sharing = non_empty_string(value, "sharing").unwrap_or_else(|| "public".to_owned());
    if !matches!(sharing.as_str(), "public" | "private") {
        return None;
    }
    let uploader = value.get("user");
    let publisher = value.get("publisher_metadata");
    let uploader_urn = uploader
        .and_then(|user| user.get("urn"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let uploader_sc_user_id = uploader
        .and_then(|user| numeric_id(user.get("id")))
        .or_else(|| {
            urn_id(
                uploader.and_then(|user| user.get("urn")),
                "soundcloud:users:",
            )
        });
    Some(HydratedTrack {
        sc_track_id: track_id.to_owned(),
        urn,
        title_normalized: catalog_normalize::normalize_title(&title),
        title,
        description: non_empty_string(value, "description"),
        genre: non_empty_string(value, "genre"),
        tags: value
            .get("tag_list")
            .and_then(Value::as_str)
            .into_iter()
            .flat_map(str::split_whitespace)
            .map(str::to_owned)
            .collect(),
        duration_ms: duration,
        artwork_url: non_empty_string(value, "artwork_url"),
        permalink_url: non_empty_string(value, "permalink_url"),
        waveform_url: non_empty_string(value, "waveform_url"),
        language: non_empty_string(value, "language"),
        isrc: publisher.and_then(|metadata| non_empty_string(metadata, "isrc")),
        metadata_artist: publisher.and_then(|metadata| non_empty_string(metadata, "artist")),
        sharing,
        sc_created_at: value
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(parse_timestamp),
        sc_last_modified: value
            .get("last_modified")
            .and_then(Value::as_str)
            .and_then(parse_timestamp),
        uploader_sc_user_id,
        uploader_urn,
        uploader_username: uploader.and_then(|user| non_empty_string(user, "username")),
        uploader_avatar_url: uploader.and_then(|user| non_empty_string(user, "avatar_url")),
        play_count_sc: nonnegative_integer(value, "playback_count"),
        likes_count_sc: nonnegative_integer(value, "likes_count"),
        reposts_count_sc: nonnegative_integer(value, "reposts_count"),
        comments_count_sc: nonnegative_integer(value, "comment_count"),
        needs_duration_resolve: duration <= 0 || (duration == 30_000 && full_duration.is_none()),
    })
}

fn non_empty_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn nonnegative_integer(value: &Value, key: &str) -> Option<i64> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0)
}

fn track_id(value: &Value) -> Result<String, PlaylistObserveError> {
    numeric_id(value.get("id"))
        .or_else(|| urn_id(value.get("urn"), "soundcloud:tracks:"))
        .ok_or(PlaylistObserveError::Invalid(
            "playlist track ID is missing",
        ))
}

fn numeric_id(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Number(value) => value.as_u64().filter(|id| *id > 0).map(|id| id.to_string()),
        Value::String(value) if canonical_numeric_id(value) => Some(value.to_owned()),
        _ => None,
    }
}

fn urn_id(value: Option<&Value>, prefix: &str) -> Option<String> {
    let id = value?.as_str()?.strip_prefix(prefix)?;
    canonical_numeric_id(id).then(|| id.to_owned())
}

fn canonical_numeric_id(value: &str) -> bool {
    value
        .parse::<u64>()
        .is_ok_and(|id| id > 0 && id.to_string() == value)
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
        .or_else(|| {
            DateTime::parse_from_str(value, "%Y/%m/%d %H:%M:%S %z")
                .ok()
                .map(|value| value.with_timezone(&Utc))
        })
}

fn validate_next_target(target: &str, expected_path: &str) -> Result<(), PlaylistObserveError> {
    let target = Url::parse(target)
        .map_err(|_| PlaylistObserveError::Invalid("playlist next link is invalid"))?;
    if target.path() != expected_path {
        return Err(PlaylistObserveError::Invalid(
            "playlist next link changed its resource",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn raw_numeric_track_ids_survive_page_parsing() {
        let page = TrackPage::parse(&json!({
            "collection": [
                { "id": 9007199254740991_u64 },
                { "id": "9007199254740992" },
                { "urn": "soundcloud:tracks:9007199254740993" }
            ],
            "next_href": null
        }))
        .unwrap();

        assert_eq!(
            page.track_ids,
            ["9007199254740991", "9007199254740992", "9007199254740993"]
        );
        assert!(page.hydrated_tracks.is_empty());
    }

    #[test]
    fn track_without_a_raw_id_is_rejected() {
        let result = TrackPage::parse(&json!({
            "collection": [{ "title": "unhydrated private track" }],
            "next_href": null
        }));

        assert!(matches!(result, Err(PlaylistObserveError::Invalid(_))));
    }

    #[test]
    fn next_link_cannot_change_the_observed_resource() {
        let result = validate_next_target(
            "https://api.soundcloud.com/users/42/tracks?offset=200",
            "/playlists/42/tracks",
        );

        assert!(matches!(result, Err(PlaylistObserveError::Invalid(_))));
    }

    #[test]
    fn state_requires_the_requested_playlist_identity() {
        let urn = PlaylistUrn::parse("soundcloud:playlists:42").unwrap();
        let result = RemoteState::parse(
            &json!({
                "id": 41,
                "user": { "id": 7 },
                "track_count": 0,
                "last_modified": "2026-08-20T10:00:00Z"
            }),
            &urn,
        );

        assert!(matches!(result, Err(PlaylistObserveError::Invalid(_))));
    }

    #[test]
    fn full_track_payload_is_ready_for_catalog_hydration() {
        let page = TrackPage::parse(&json!({
            "collection": [{
                "id": "9007199254740992",
                "urn": "soundcloud:tracks:9007199254740992",
                "title": "Thé ᴍᴏᴏɴ",
                "duration": 123456,
                "sharing": "public",
                "user": { "id": 42, "username": "artist" }
            }],
            "next_href": null
        }))
        .unwrap();

        assert_eq!(page.hydrated_tracks.len(), 1);
        assert_eq!(page.hydrated_tracks[0].title_normalized, "moon");
    }

    #[test]
    fn aggregate_response_budget_is_bounded() {
        let mut budget = BodyBudget::default();

        assert!(budget.charge(MAX_OBSERVATION_BYTES).is_ok());
        assert!(budget.charge(1).is_err());
    }
}
