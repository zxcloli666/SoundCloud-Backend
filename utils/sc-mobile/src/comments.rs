use serde_json::json;

use crate::request::MobileRequest;

pub fn post(track_urn: &str, body: &str, track_time_ms: u64) -> MobileRequest {
    MobileRequest::post(
        &["tracks", track_urn, "comments"],
        &json!({ "body": body, "track_time": track_time_ms }),
    )
}

pub fn list(track_urn: &str) -> MobileRequest {
    MobileRequest::get(&["tracks", track_urn, "comments"])
}

pub fn delete(comment_urn: &str) -> MobileRequest {
    MobileRequest::delete(&["comments", comment_urn])
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use wreq::Method;

    use super::*;
    use crate::request::Kind;

    #[test]
    fn a_comment_is_posted_to_its_track_and_deleted_by_its_urn() {
        let posted = post("soundcloud:tracks:1", "probe", 0);
        let body: Option<Value> = posted
            .body()
            .and_then(|body| serde_json::from_slice(body).ok());

        assert_eq!(posted.method(), &Method::POST);
        assert_eq!(posted.kind(), Kind::Write);
        assert_eq!(
            posted
                .url()
                .ok()
                .map(|url| url.path().to_owned())
                .as_deref(),
            Some("/tracks/soundcloud:tracks:1/comments")
        );
        assert_eq!(body, Some(json!({ "body": "probe", "track_time": 0 })));

        let deleted = delete("soundcloud:comments:9");
        assert_eq!(deleted.method(), &Method::DELETE);
        assert_eq!(deleted.kind(), Kind::Write);
        assert_eq!(
            deleted
                .url()
                .ok()
                .map(|url| url.path().to_owned())
                .as_deref(),
            Some("/comments/soundcloud:comments:9")
        );
    }
}
