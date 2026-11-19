use serde_json::json;

use crate::request::MobileRequest;

pub fn follow(user_urn: &str) -> MobileRequest {
    MobileRequest::post(&["follows", "users", user_urn], &json!({}))
}

pub fn unfollow(user_urn: &str) -> MobileRequest {
    MobileRequest::delete(&["follows", "users", user_urn])
}

pub fn followings() -> MobileRequest {
    MobileRequest::get(&["you", "followings"])
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
    fn a_follow_names_the_user_in_the_path() {
        let followed = follow("soundcloud:users:7");
        let body: Option<Value> = followed
            .body()
            .and_then(|body| serde_json::from_slice(body).ok());

        assert_eq!(followed.method(), &Method::POST);
        assert_eq!(
            path(&followed).as_deref(),
            Some("/follows/users/soundcloud:users:7")
        );
        assert_eq!(body, Some(json!({})));

        let unfollowed = unfollow("soundcloud:users:7");
        assert_eq!(unfollowed.method(), &Method::DELETE);
        assert_eq!(unfollowed.body(), None);
        assert_eq!(
            path(&unfollowed).as_deref(),
            Some("/follows/users/soundcloud:users:7")
        );
    }

    #[test]
    fn the_followings_are_read_from_you() {
        let read = followings();

        assert_eq!(read.method(), &Method::GET);
        assert_eq!(path(&read).as_deref(), Some("/you/followings"));
    }
}
