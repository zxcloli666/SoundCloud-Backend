use backend_contracts::{CatalogEntity, CatalogRefreshPayload};
use serde_json::Value;
use url::Url;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct EntityKey {
    pub entity: CatalogEntity,
    pub id: String,
}

impl EntityKey {
    pub fn urn(&self) -> String {
        self.entity.urn(&self.id)
    }

    pub fn parse(urn: &str) -> Option<Self> {
        let (namespace, id) = urn.strip_prefix("soundcloud:")?.split_once(':')?;
        let entity = match namespace {
            "tracks" => CatalogEntity::Track,
            "playlists" => CatalogEntity::Playlist,
            "users" => CatalogEntity::User,
            _ => return None,
        };
        let payload = CatalogRefreshPayload {
            entity,
            sc_id: id.to_owned(),
            owner_id: None,
        };
        payload.is_valid().then(|| Self {
            entity,
            id: id.to_owned(),
        })
    }

    pub fn from_payload(value: &Value) -> AppResult<Self> {
        let namespace = match value.get("kind").and_then(Value::as_str) {
            Some("track") => "tracks",
            Some("playlist") => "playlists",
            Some("user") => "users",
            _ => return Err(invalid_payload()),
        };
        let id = match value.get("id") {
            Some(Value::String(id)) => id.clone(),
            Some(Value::Number(id)) => id.to_string(),
            _ => return Err(invalid_payload()),
        };
        let urn = format!("soundcloud:{namespace}:{id}");
        if value
            .get("urn")
            .is_some_and(|value| value.as_str() != Some(&urn))
        {
            return Err(invalid_payload());
        }
        Self::parse(&urn).ok_or_else(invalid_payload)
    }
}

fn invalid_payload() -> AppError {
    AppError::coded(
        axum::http::StatusCode::BAD_GATEWAY,
        "invalid_resolve_response",
        "SoundCloud returned an invalid entity",
    )
}

pub(super) struct ResolveInput {
    pub upstream: String,
    pub entity: Option<EntityKey>,
    pub permalinks: Vec<String>,
    pub requires_upstream: bool,
    pub short_link: bool,
}

impl ResolveInput {
    pub fn parse(raw: &str) -> AppResult<Self> {
        let raw = raw.trim();
        if raw.is_empty() || raw.len() > 4096 {
            return Err(AppError::bad_request("url must contain 1 to 4096 bytes"));
        }
        if let Some(entity) = EntityKey::parse(raw) {
            return Ok(Self {
                upstream: raw.to_owned(),
                entity: Some(entity),
                permalinks: Vec::new(),
                requires_upstream: false,
                short_link: false,
            });
        }
        let mut url =
            Url::parse(raw).map_err(|_| AppError::bad_request("Invalid SoundCloud URL"))?;
        let main_host = matches!(
            url.host_str(),
            Some("soundcloud.com" | "www.soundcloud.com" | "m.soundcloud.com")
        );
        let short_link = matches!(url.host_str(), Some("on.soundcloud.com" | "snd.sc"));
        if !matches!(url.scheme(), "http" | "https")
            || (!main_host && !short_link)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
        {
            return Err(AppError::bad_request(
                "Expected a SoundCloud URL or canonical entity URN",
            ));
        }
        url.set_fragment(None);
        let segments: Vec<_> = url.path_segments().into_iter().flatten().collect();
        let secret_path = matches!(segments.as_slice(), [_, _, secret] if secret.starts_with("s-"))
            || matches!(segments.as_slice(), [_, "sets", _, secret] if secret.starts_with("s-"));
        let requires_upstream = secret_path
            || url
                .query_pairs()
                .any(|(key, _)| key != "si" && !key.starts_with("utm_"));
        let mut permalinks = Vec::new();
        if !requires_upstream {
            url.set_query(None);
            if main_host {
                let path = url.path().trim_end_matches('/').to_owned();
                for scheme in ["https", "http"] {
                    for host in ["soundcloud.com", "www.soundcloud.com", "m.soundcloud.com"] {
                        permalinks.push(format!("{scheme}://{host}{path}"));
                        permalinks.push(format!("{scheme}://{host}{path}/"));
                    }
                }
                url.set_scheme("https")
                    .map_err(|_| AppError::bad_request("Invalid URL scheme"))?;
                url.set_host(Some("soundcloud.com"))
                    .map_err(|_| AppError::bad_request("Invalid URL host"))?;
                url.set_path(&path);
            }
        }
        Ok(Self {
            upstream: url.into(),
            entity: None,
            permalinks,
            requires_upstream,
            short_link,
        })
    }
}
