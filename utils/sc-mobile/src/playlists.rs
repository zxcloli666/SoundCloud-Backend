use serde_json::json;

use crate::request::MobileRequest;

pub fn create(title: &str) -> MobileRequest {
    MobileRequest::post(
        &["playlists"],
        &json!({ "title": title, "sharing": "private", "tracks": [] }),
    )
}

pub fn read(playlist_urn: &str) -> MobileRequest {
    MobileRequest::get(&["playlists", playlist_urn])
}

pub fn add_track(playlist_urn: &str, track_urn: &str) -> MobileRequest {
    MobileRequest::post(
        &["playlists", playlist_urn, "tracks"],
        &json!({ "track_urn": track_urn }),
    )
}

pub fn remove_track(playlist_urn: &str, track_urn: &str) -> MobileRequest {
    MobileRequest::delete(&["playlists", playlist_urn, "tracks", track_urn])
}

pub fn delete(playlist_urn: &str) -> MobileRequest {
    MobileRequest::delete(&["playlists", playlist_urn])
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use wreq::Method;

    use super::*;

    fn shape(request: &MobileRequest) -> (Method, Option<String>, Option<Value>) {
        (
            request.method().clone(),
            request.url().ok().map(|url| url.path().to_owned()),
            request
                .body()
                .and_then(|body| serde_json::from_slice(body).ok()),
        )
    }

    #[test]
    fn a_private_playlist_is_created_with_a_title() {
        assert_eq!(
            shape(&create("probe")),
            (
                Method::POST,
                Some("/playlists".to_owned()),
                Some(json!({ "title": "probe", "sharing": "private", "tracks": [] }))
            )
        );
    }

    #[test]
    fn a_track_goes_into_and_out_of_a_playlist() {
        let playlist = "soundcloud:playlists:5";
        let track = "soundcloud:tracks:1";

        assert_eq!(
            shape(&add_track(playlist, track)),
            (
                Method::POST,
                Some("/playlists/soundcloud:playlists:5/tracks".to_owned()),
                Some(json!({ "track_urn": track }))
            )
        );
        assert_eq!(
            shape(&remove_track(playlist, track)),
            (
                Method::DELETE,
                Some("/playlists/soundcloud:playlists:5/tracks/soundcloud:tracks:1".to_owned()),
                None
            )
        );
    }

    #[test]
    fn a_playlist_is_read_and_deleted_by_its_urn() {
        let playlist = "soundcloud:playlists:5";

        assert_eq!(
            shape(&read(playlist)),
            (
                Method::GET,
                Some("/playlists/soundcloud:playlists:5".to_owned()),
                None
            )
        );
        assert_eq!(
            shape(&delete(playlist)),
            (
                Method::DELETE,
                Some("/playlists/soundcloud:playlists:5".to_owned()),
                None
            )
        );
    }
}
