use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
pub struct ScTokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    pub expires_in: i64,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub token_type: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ScMe {
    pub urn: String,
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub country_code: Option<String>,
    #[serde(flatten)]
    pub rest: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayRead<T> {
    Found(T),
    Missing,
    Unavailable,
}

impl<T> RelayRead<T> {
    pub fn found(self) -> Option<T> {
        match self {
            Self::Found(value) => Some(value),
            Self::Missing | Self::Unavailable => None,
        }
    }

    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable)
    }

    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
}
