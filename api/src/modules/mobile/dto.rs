use chrono::{DateTime, Utc};
use redact::Secret;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkStarted {
    pub id: Uuid,
    pub url: String,
    pub account: LinkAccount,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkAccount {
    pub username: String,
    pub avatar_url: Option<String>,
}

#[derive(Deserialize)]
pub struct LinkCallback {
    pub id: Uuid,
    pub code: Secret<String>,
    pub state: Secret<String>,
}

#[derive(Deserialize)]
pub struct LinkStatusQuery {
    pub id: Uuid,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct LinkStatus {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MobileOverview {
    pub available: bool,
    pub linked: bool,
    pub status: Option<String>,
    pub plan: Option<String>,
    pub plan_seen_at: Option<DateTime<Utc>>,
    pub uq_opt_in: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UqChoice {
    pub opt_in: bool,
}
