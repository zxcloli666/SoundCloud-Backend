use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use url::Url;

use super::actions::ActionError;
use super::client::SoundCloudClient;
use super::model::ClaimedMutation;

const LISTING_PATH: &str = "/me/playlists";
const FIRST_PAGE: &str = "/me/playlists?show_tracks=false&linked_partitioning=true&limit=50";
const MAX_PAGES: usize = 4;
const CLOCK_SLACK_MINUTES: i64 = 5;

pub async fn confirm(
    client: &SoundCloudClient,
    access_token: &str,
    mutation: &ClaimedMutation,
    owned: &[String],
) -> Result<Option<Value>, ActionError> {
    let title = mutation
        .payload
        .as_ref()
        .and_then(|payload| payload.pointer("/playlist/title"))
        .and_then(Value::as_str)
        .ok_or(ActionError::InvalidPayload("playlist title is missing"))?;
    let mut listing = Vec::new();
    let mut next = Some(FIRST_PAGE.to_owned());
    for _ in 0..MAX_PAGES {
        let Some(path) = next.take() else {
            break;
        };
        let page = client.get(&path, access_token).await?;
        let playlists = page.get("collection").and_then(Value::as_array).ok_or(
            ActionError::InvalidRemoteResult("playlist listing has no collection"),
        )?;
        listing.extend(playlists.iter().cloned());
        next = page
            .get("next_href")
            .and_then(Value::as_str)
            .and_then(listing_page);
    }
    let not_before = mutation.created_at - Duration::minutes(CLOCK_SLACK_MINUTES);
    let mut created = find_created(&listing, title, not_before, owned);
    if let Some(playlist) = created.as_mut().and_then(Value::as_object_mut) {
        playlist.remove("tracks");
    }
    Ok(created)
}

pub fn find_created(
    listing: &[Value],
    title: &str,
    not_before: DateTime<Utc>,
    owned: &[String],
) -> Option<Value> {
    listing
        .iter()
        .filter(|playlist| {
            playlist
                .get("title")
                .and_then(Value::as_str)
                .is_some_and(|listed| listed.trim() == title.trim())
        })
        .filter(|playlist| {
            playlist
                .get("urn")
                .and_then(Value::as_str)
                .is_some_and(|urn| !owned.iter().any(|known| known == urn))
        })
        .filter_map(|playlist| {
            Some((
                catalog_ingest::parse_dt(playlist.get("created_at"))?,
                playlist,
            ))
        })
        .filter(|(created_at, _)| *created_at >= not_before)
        .min_by_key(|(created_at, _)| *created_at)
        .map(|(_, playlist)| playlist.clone())
}

fn listing_page(href: &str) -> Option<String> {
    let url = Url::parse(href).ok()?;
    let opaque_collection = url.path() == "/collection"
        && url
            .query_pairs()
            .any(|(key, value)| key == "cursor" && !value.is_empty());
    if url.path() != LISTING_PATH && !opaque_collection {
        return None;
    }
    Some(format!("{}?{}", url.path(), url.query()?))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn at(minute: u32) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&format!("2026-10-17T10:{minute:02}:00Z"))
            .map(|time| time.with_timezone(&Utc))
            .unwrap_or_default()
    }

    fn listed(id: u32, title: &str, created_at: &str) -> Value {
        json!({
            "urn": format!("soundcloud:playlists:{id}"),
            "title": title,
            "created_at": created_at
        })
    }

    #[test]
    fn a_newer_playlist_with_the_same_title_is_the_created_one() {
        let listing = [
            listed(1, "Other", "2026/10/17 10:20:00 +0000"),
            listed(2, " Yandex Music ", "2026/10/17 10:20:00 +0000"),
        ];

        let created = find_created(&listing, "Yandex Music", at(15), &[]);

        assert_eq!(created, Some(listing[1].clone()));
    }

    #[test]
    fn a_playlist_older_than_the_queued_create_is_ignored() {
        let listing = [listed(1, "Mix", "2026-10-17T10:10:00Z")];

        assert_eq!(find_created(&listing, "Mix", at(15), &[]), None);
    }

    #[test]
    fn a_playlist_the_user_already_owned_is_ignored() {
        let listing = [listed(1, "Mix", "2026-10-17T10:20:00Z")];
        let owned = ["soundcloud:playlists:1".to_owned()];

        assert_eq!(find_created(&listing, "Mix", at(15), &owned), None);
    }

    #[test]
    fn the_oldest_of_several_matches_wins() {
        let listing = [
            listed(3, "Mix", "2026-10-17T10:40:00Z"),
            listed(2, "Mix", "2026-10-17T10:20:00Z"),
            listed(1, "Mix", "2026-10-17T10:30:00Z"),
        ];

        assert_eq!(
            find_created(&listing, "Mix", at(15), &[]),
            Some(listing[1].clone())
        );
    }

    #[test]
    fn only_listing_pages_of_the_same_resource_are_followed() {
        assert_eq!(
            listing_page("https://api.soundcloud.com/me/playlists?cursor=abc&limit=50").as_deref(),
            Some("/me/playlists?cursor=abc&limit=50")
        );
        assert_eq!(
            listing_page("https://api.soundcloud.com/collection?page_size=50&cursor=123")
                .as_deref(),
            Some("/collection?page_size=50&cursor=123")
        );
        assert_eq!(
            listing_page("https://api.soundcloud.com/users/1/playlists?cursor=abc"),
            None
        );
        assert_eq!(
            listing_page("https://api.soundcloud.com/collection?page_size=50"),
            None
        );
    }
}
