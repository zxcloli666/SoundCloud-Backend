use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::modules::oauth_apps::model::{ClientKind, OAuthApp};

#[derive(Deserialize)]
pub struct CreateOAuthAppDto {
    pub name: String,
    #[serde(rename = "clientId")]
    pub client_id: String,
    #[serde(rename = "clientSecret")]
    pub client_secret: String,
    #[serde(rename = "redirectUri")]
    pub redirect_uri: String,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default, rename = "clientKind")]
    pub client_kind: ClientKind,
    #[serde(default, rename = "appId")]
    pub app_id: Option<String>,
    #[serde(default, rename = "appVersion")]
    pub app_version: Option<String>,
    #[serde(default, rename = "userAgent")]
    pub user_agent: Option<String>,
}

#[derive(Default, Deserialize)]
pub struct UpdateOAuthAppDto {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "clientId")]
    pub client_id: Option<String>,
    #[serde(default, rename = "clientSecret")]
    pub client_secret: Option<String>,
    #[serde(default, rename = "redirectUri")]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default, rename = "clientKind")]
    pub client_kind: Option<ClientKind>,
    #[serde(default, rename = "appId")]
    pub app_id: Option<String>,
    #[serde(default, rename = "appVersion")]
    pub app_version: Option<String>,
    #[serde(default, rename = "userAgent")]
    pub user_agent: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OAuthAppResponse {
    pub id: Uuid,
    pub name: String,
    #[serde(rename = "clientId")]
    pub client_id: String,
    #[serde(rename = "redirectUri")]
    pub redirect_uri: String,
    pub active: bool,
    #[serde(rename = "clientKind")]
    pub client_kind: String,
    #[serde(rename = "appId")]
    pub app_id: Option<String>,
    #[serde(rename = "appVersion")]
    pub app_version: Option<String>,
    #[serde(rename = "userAgent")]
    pub user_agent: Option<String>,
    #[serde(rename = "lastUsedAt")]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(rename = "createdAt")]
    pub created_at: NaiveDateTime,
}

impl From<OAuthApp> for OAuthAppResponse {
    fn from(app: OAuthApp) -> Self {
        Self {
            id: app.id,
            name: app.name,
            client_id: app.client_id,
            redirect_uri: app.redirect_uri,
            active: app.active,
            client_kind: app.client_kind,
            app_id: app.app_id,
            app_version: app.app_version,
            user_agent: app.user_agent,
            last_used_at: app.last_used_at,
            created_at: app.created_at,
        }
    }
}
