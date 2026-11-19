use serde::Deserialize;

use crate::request::MobileRequest;

const USER_URN_PREFIX: &str = "soundcloud:users:";

#[derive(Deserialize)]
struct MeBody {
    user: Option<MeUser>,
}

#[derive(Deserialize)]
struct MeUser {
    urn: Option<String>,
}

pub fn me() -> MobileRequest {
    MobileRequest::get(&["me"])
}

pub fn user_id(body: &[u8]) -> Option<String> {
    let urn = serde_json::from_slice::<MeBody>(body).ok()?.user?.urn?;
    let id = urn.strip_prefix(USER_URN_PREFIX)?;
    (!id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())).then(|| id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_account_id_comes_from_the_user_urn() {
        let body = br#"{"configuration":{},"primary_email_confirmed":true,"user":{"urn":"soundcloud:users:4242","username":"fake-listener","permalink":"fake-listener"}}"#;

        assert_eq!(user_id(body).as_deref(), Some("4242"));
    }

    #[test]
    fn an_answer_without_a_user_urn_names_no_account() {
        for body in [
            br#"{"user":{"urn":"soundcloud:tracks:4242"}}"#.as_slice(),
            br#"{"user":{"urn":"soundcloud:users:"}}"#,
            br#"{"user":{"urn":"soundcloud:users:12a"}}"#,
            br#"{"user":{}}"#,
            br#"{}"#,
            b"not json",
        ] {
            assert_eq!(user_id(body), None);
        }
    }
}
