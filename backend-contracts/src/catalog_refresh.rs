use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogEntity {
    Track,
    Playlist,
    User,
    Profile,
    WebProfiles,
}

impl CatalogEntity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Track => "track",
            Self::Playlist => "playlist",
            Self::User => "user",
            Self::Profile => "profile",
            Self::WebProfiles => "web_profiles",
        }
    }

    pub fn path(self, sc_id: &str) -> String {
        match self {
            Self::Track => format!("/tracks/{sc_id}"),
            Self::Playlist => format!("/playlists/{sc_id}"),
            Self::User => format!("/users/{sc_id}"),
            Self::Profile => "/me".to_owned(),
            Self::WebProfiles => format!("/users/soundcloud:users:{sc_id}/web-profiles?limit=200"),
        }
    }

    pub fn urn(self, sc_id: &str) -> String {
        let segment = match self {
            Self::Track => "tracks",
            Self::Playlist => "playlists",
            Self::User | Self::Profile | Self::WebProfiles => "users",
        };
        format!("soundcloud:{segment}:{sc_id}")
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogRefreshPayload {
    pub entity: CatalogEntity,
    pub sc_id: String,
    pub owner_id: Option<String>,
}

impl CatalogRefreshPayload {
    pub fn is_valid(&self) -> bool {
        let valid_id = |id: &str| {
            !id.is_empty()
                && id.bytes().all(|byte| byte.is_ascii_digit())
                && id
                    .parse::<i64>()
                    .is_ok_and(|value| value > 0 && value.to_string() == id)
        };
        valid_id(&self.sc_id)
            && self.owner_id.as_deref().is_none_or(valid_id)
            && (self.entity != CatalogEntity::WebProfiles || self.owner_id.is_none())
            && (self.entity != CatalogEntity::Profile
                || self.owner_id.as_deref() == Some(&self.sc_id))
    }

    pub fn dedup_key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.entity.as_str(),
            self.sc_id,
            self.owner_id.as_deref().unwrap_or("public")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_profiles_are_shared_public_refreshes_with_a_bounded_official_path() {
        let mut payload = CatalogRefreshPayload {
            entity: CatalogEntity::WebProfiles,
            sc_id: "42".into(),
            owner_id: None,
        };
        assert!(payload.is_valid());
        assert_eq!(payload.dedup_key(), "web_profiles:42:public");
        assert_eq!(
            payload.entity.path("42"),
            "/users/soundcloud:users:42/web-profiles?limit=200"
        );
        payload.owner_id = Some("42".into());
        assert!(!payload.is_valid());
    }

    #[test]
    fn a_profile_refresh_cannot_select_another_accounts_token() {
        let mut payload = CatalogRefreshPayload {
            entity: CatalogEntity::Profile,
            sc_id: "17".into(),
            owner_id: Some("18".into()),
        };
        assert!(!payload.is_valid());
        payload.owner_id = Some("17".into());
        assert!(payload.is_valid());
        payload.sc_id = "17/likes".into();
        assert!(!payload.is_valid());
    }

    #[test]
    fn public_and_owner_observations_have_distinct_deduplication_keys() {
        let public = CatalogRefreshPayload {
            entity: CatalogEntity::Track,
            sc_id: "42".into(),
            owner_id: None,
        };
        let mut owner = public.clone();
        owner.owner_id = Some("17".into());
        assert_ne!(public.dedup_key(), owner.dedup_key());
    }
}
