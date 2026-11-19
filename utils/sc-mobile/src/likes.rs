use serde_json::json;

use crate::request::MobileRequest;

pub fn create(track_urn: &str) -> MobileRequest {
    MobileRequest::post(
        &["likes", "tracks", "create"],
        &json!({ "track_urn": track_urn }),
    )
}

pub fn delete(track_urn: &str) -> MobileRequest {
    MobileRequest::post(
        &["likes", "tracks", "delete"],
        &json!({ "track_urn": track_urn }),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::Value;
    use wreq::Method;

    use super::*;
    use crate::request::Kind;

    fn shape(request: &MobileRequest) -> (Method, Kind, Option<String>, Option<Value>) {
        (
            request.method().clone(),
            request.kind(),
            request.url().ok().map(|url| url.path().to_owned()),
            request
                .body()
                .and_then(|body| serde_json::from_slice(body).ok()),
        )
    }

    #[test]
    fn a_like_is_created_and_deleted_by_posting_the_track() {
        let body = Some(json!({ "track_urn": "soundcloud:tracks:1" }));

        assert_eq!(
            shape(&create("soundcloud:tracks:1")),
            (
                Method::POST,
                Kind::Write,
                Some("/likes/tracks/create".to_owned()),
                body.clone()
            )
        );
        assert_eq!(
            shape(&delete("soundcloud:tracks:1")),
            (
                Method::POST,
                Kind::Write,
                Some("/likes/tracks/delete".to_owned()),
                body
            )
        );
    }
}
