use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    #[default]
    Default,
    Mobile,
}

impl ClientKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Mobile => "mobile",
        }
    }
}

#[derive(Clone, FromRow, Serialize)]
pub struct OAuthApp {
    pub id: Uuid,
    pub name: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub active: bool,
    pub client_kind: String,
    pub app_id: Option<String>,
    pub app_version: Option<String>,
    pub user_agent: Option<String>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl std::fmt::Debug for OAuthApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthApp")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("client_id", &self.client_id)
            .field("client_secret", &redact::MASK)
            .field("redirect_uri", &self.redirect_uri)
            .field("active", &self.active)
            .field("client_kind", &self.client_kind)
            .field("app_id", &self.app_id)
            .field("app_version", &self.app_version)
            .field("user_agent", &self.user_agent)
            .field("last_used_at", &self.last_used_at)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}
