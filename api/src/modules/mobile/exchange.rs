use chrono::{DateTime, TimeDelta, Utc};
use redact::Secret;
use sc_mobile::{Caller, MobileClient, MobileError, MobileReply, UserPlan, me};

const FALLBACK_LIFETIME_SECONDS: i64 = 60 * 60;
const MAX_LIFETIME_SECONDS: i64 = 30 * 24 * 60 * 60;
const INVALID_GRANT: &str = "invalid_grant";
const INVALID_CLIENT: &str = "invalid_client";
const SUCCESS_CLASS: &str = "2xx";

pub(super) struct Linked {
    pub(super) soundcloud_user_id: String,
    pub(super) access_token: Secret<String>,
    pub(super) refresh_token: Secret<String>,
    pub(super) expires_at: DateTime<Utc>,
    pub(super) scope: String,
    pub(super) plan: Option<UserPlan>,
}

pub(super) enum Exchanged {
    Account(Linked),
    Failed(&'static str),
}

pub(super) async fn exchange(
    client: &MobileClient,
    code: &Secret<String>,
    verifier: &Secret<String>,
    device: &str,
) -> Exchanged {
    let tokens = client.exchange_code(code, verifier, device).await;
    let class = tokens
        .as_ref()
        .map_or_else(MobileError::status_class, |_| SUCCESS_CLASS);
    crate::metrics::record_mobile_request("token", class);
    let tokens = match tokens {
        Ok(tokens) => tokens,
        Err(error) => return Exchanged::Failed(exchange_failure(&error)),
    };
    let Some(refresh_token) = tokens.refresh_token else {
        return Exchanged::Failed("exchange_failed");
    };
    let caller = Caller::user(device, tokens.access_token.clone());
    let reply = client.call(&caller, &me::me()).await;
    let class = reply
        .as_ref()
        .map_or_else(MobileError::status_class, MobileReply::status_class);
    crate::metrics::record_mobile_request("me", class);
    let reply = match reply {
        Ok(reply) => reply,
        Err(error) => return Exchanged::Failed(profile_unreachable(&error)),
    };
    if let Some(reason) = profile_refusal(&reply) {
        return Exchanged::Failed(reason);
    }
    let Some(soundcloud_user_id) = me::user_id(&reply.body) else {
        return Exchanged::Failed("profile_invalid");
    };
    Exchanged::Account(Linked {
        soundcloud_user_id,
        access_token: tokens.access_token,
        refresh_token,
        expires_at: expiry(tokens.expires_in),
        scope: tokens.scope.unwrap_or_default(),
        plan: reply.plan,
    })
}

fn exchange_failure(error: &MobileError) -> &'static str {
    match error {
        MobileError::Captcha => "captcha",
        MobileError::Refused { error, .. } => match error.as_deref() {
            Some(INVALID_GRANT) => "code_rejected",
            Some(INVALID_CLIENT) => "profile_unavailable",
            _ => "exchange_failed",
        },
        MobileError::Transport(_) | MobileError::TooLarge(_) => "unreachable",
        _ => "exchange_failed",
    }
}

fn profile_unreachable(error: &MobileError) -> &'static str {
    if error.is_transport() {
        "unreachable"
    } else {
        "profile_failed"
    }
}

fn profile_refusal(reply: &MobileReply) -> Option<&'static str> {
    if reply.is_captcha() {
        return Some("captcha");
    }
    if reply.status == 401 {
        return Some("profile_rejected");
    }
    (!reply.is_success()).then_some("profile_failed")
}

fn expiry(expires_in: Option<i64>) -> DateTime<Utc> {
    let seconds = expires_in
        .filter(|seconds| *seconds > 0)
        .unwrap_or(FALLBACK_LIFETIME_SECONDS)
        .min(MAX_LIFETIME_SECONDS);
    let now = Utc::now();
    TimeDelta::try_seconds(seconds)
        .and_then(|lifetime| now.checked_add_signed(lifetime))
        .unwrap_or(now)
}
