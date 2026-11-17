use serde::Deserialize;

use super::{Fetch, Verdict, headers, unavailable};

const STATUS: &str = "https://api.music.yandex.net/account/status";
const RUSSIAN_EXIT: &str = "geo-ru";

#[derive(Deserialize)]
struct Reply {
    result: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    account: Option<Account>,
}

#[derive(Deserialize)]
struct Account {
    uid: Option<serde_json::Value>,
}

impl Reply {
    fn has_account(&self) -> bool {
        self.result
            .as_ref()
            .and_then(|payload| payload.account.as_ref())
            .is_some_and(|account| account.uid.is_some())
    }
}

pub(super) async fn check<F: Fetch>(fetch: &F, token: &str) -> Verdict {
    if token.is_empty() {
        return Verdict::Rejected("the token is empty".to_owned());
    }
    let authorization = format!("OAuth {token}");
    let request = match headers(&[
        ("authorization", &authorization),
        ("x-egress-tag", RUSSIAN_EXIT),
        ("accept", "application/json"),
        ("user-agent", "Yandex-Music-API"),
    ]) {
        Ok(request) => request,
        Err(verdict) => return verdict,
    };
    match fetch.get(STATUS, request).await {
        Ok(body) => match serde_json::from_slice::<Reply>(&body) {
            Ok(reply) if reply.has_account() => Verdict::Valid,
            Ok(_) => Verdict::Rejected("Yandex returned no account for the token".to_owned()),
            Err(_) => Verdict::Unavailable("Yandex answered with an unexpected body".to_owned()),
        },
        Err(error) if error.status_code() == Some(401) => {
            Verdict::Rejected("Yandex rejected the token (401)".to_owned())
        }
        Err(error) => unavailable("Yandex Music", &error),
    }
}
