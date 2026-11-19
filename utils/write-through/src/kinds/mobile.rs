use std::fmt;

use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use super::Kind;

#[derive(Serialize)]
pub struct MobileCredentialLink<'a> {
    pub soundcloud_user_id: &'a str,
    pub oauth_app_id: Uuid,
    pub device_id: Uuid,
    pub access_token: &'a str,
    pub refresh_token: &'a str,
    pub expires_at: DateTime<Utc>,
    pub scope: &'a str,
    pub link_v: i64,
    pub at: DateTime<Utc>,
}

impl fmt::Debug for MobileCredentialLink<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MobileCredentialLink")
            .field("oauth_app_id", &self.oauth_app_id)
            .field("link_v", &self.link_v)
            .field("at", &self.at)
            .finish_non_exhaustive()
    }
}

impl Kind for MobileCredentialLink<'_> {
    const NAME: &'static str = "mobile_credential.link";
    type Output = bool;
}

#[derive(Serialize)]
pub struct MobileCredentialDrop<'a> {
    pub soundcloud_user_id: &'a str,
}

impl fmt::Debug for MobileCredentialDrop<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MobileCredentialDrop")
            .finish_non_exhaustive()
    }
}

impl Kind for MobileCredentialDrop<'_> {
    const NAME: &'static str = "mobile_credential.drop";
    type Output = Option<bool>;
}
