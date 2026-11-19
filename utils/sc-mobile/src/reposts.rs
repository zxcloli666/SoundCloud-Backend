use serde_json::json;

use crate::request::MobileRequest;

pub fn add(track_urn: &str) -> MobileRequest {
    MobileRequest::post(&["reposts", "tracks", track_urn], &json!({}))
}

pub fn add_put(track_urn: &str) -> MobileRequest {
    MobileRequest::put(&["reposts", "tracks", track_urn], &json!({}))
}

pub fn remove(track_urn: &str) -> MobileRequest {
    MobileRequest::delete(&["reposts", "tracks", track_urn])
}

pub fn posts() -> MobileRequest {
    MobileRequest::get(&["you", "posts_and_reposts", "tracks"])
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use wreq::Method;

    use super::*;

    fn path(request: &MobileRequest) -> Option<String> {
        request.url().ok().map(|url| url.path().to_owned())
    }

    #[test]
    fn a_repost_names_the_track_in_the_path() {
        let added = add("soundcloud:tracks:1");
        let body: Option<Value> = added
            .body()
            .and_then(|body| serde_json::from_slice(body).ok());

        assert_eq!(added.method(), &Method::POST);
        assert_eq!(
            path(&added).as_deref(),
            Some("/reposts/tracks/soundcloud:tracks:1")
        );
        assert_eq!(body, Some(json!({})));

        let removed = remove("soundcloud:tracks:1");
        assert_eq!(removed.method(), &Method::DELETE);
        assert_eq!(removed.body(), None);
        assert_eq!(
            path(&removed).as_deref(),
            Some("/reposts/tracks/soundcloud:tracks:1")
        );
    }

    #[test]
    fn a_repost_can_also_be_put_on_the_same_path() {
        let added = add_put("soundcloud:tracks:1");

        assert_eq!(added.method(), &Method::PUT);
        assert_eq!(
            path(&added).as_deref(),
            Some("/reposts/tracks/soundcloud:tracks:1")
        );
    }

    #[test]
    fn the_own_posts_are_read_from_you() {
        let read = posts();

        assert_eq!(read.method(), &Method::GET);
        assert_eq!(
            path(&read).as_deref(),
            Some("/you/posts_and_reposts/tracks")
        );
    }
}
