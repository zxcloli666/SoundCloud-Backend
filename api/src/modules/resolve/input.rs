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

const SECRET_TOKEN: &str = "secret_token";

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
        let short_link = url.host_str() == Some("on.soundcloud.com");
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
        let kept: Vec<(String, String)> = url
            .query_pairs()
            .filter(|(key, _)| key == SECRET_TOKEN)
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        if kept.is_empty() {
            url.set_query(None);
        } else {
            url.query_pairs_mut().clear().extend_pairs(&kept);
        }
        let mut segments: Vec<String> = url
            .path_segments()
            .into_iter()
            .flatten()
            .filter(|segment| !segment.is_empty())
            .map(str::to_owned)
            .collect();
        let secret_at = secret_segment(&segments);
        let requires_upstream = secret_at.is_some() || !kept.is_empty();
        let mut permalinks = Vec::new();
        if main_host {
            for (index, segment) in segments.iter_mut().enumerate() {
                if Some(index) != secret_at {
                    *segment = segment.to_lowercase();
                }
            }
            let path = format!("/{}", segments.join("/"));
            if !requires_upstream {
                for scheme in ["https", "http"] {
                    for host in ["soundcloud.com", "www.soundcloud.com", "m.soundcloud.com"] {
                        permalinks.push(format!("{scheme}://{host}{path}"));
                        permalinks.push(format!("{scheme}://{host}{path}/"));
                    }
                }
            }
            url.set_scheme("https")
                .map_err(|_| AppError::bad_request("Invalid URL scheme"))?;
            url.set_host(Some("soundcloud.com"))
                .map_err(|_| AppError::bad_request("Invalid URL host"))?;
            url.set_path(&path);
        } else {
            url.set_scheme("https")
                .map_err(|_| AppError::bad_request("Invalid URL scheme"))?;
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

fn secret_segment(segments: &[String]) -> Option<usize> {
    match segments {
        [_, middle, secret] if !middle.eq_ignore_ascii_case("sets") && secret.starts_with("s-") => {
            Some(2)
        }
        [_, sets, _, secret] if sets.eq_ignore_ascii_case("sets") && secret.starts_with("s-") => {
            Some(3)
        }
        _ => None,
    }
}
