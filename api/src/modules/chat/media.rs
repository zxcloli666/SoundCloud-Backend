use std::fs;
use std::path::Path;

use redact::Secret;

use crate::config::env_opt;

pub mod backdrop;
pub mod commit;
pub mod delete;
pub mod hooks;
pub mod ledger;
pub mod limits;
pub mod mine;
pub mod refs;
pub mod reserve;
pub mod restore;
pub mod star;
pub mod urls;
pub mod usage;
pub mod view;
pub mod wire;

pub use commit::commit_media;
pub use delete::delete_media;
pub use mine::media_mine;
pub use reserve::reserve_media;
pub use restore::restore_media;
pub use urls::media_urls;
pub use usage::media_usage;

pub const SECRET_PREFIX: &str = "s3-secret-";

#[derive(Clone, Debug)]
pub struct StoreConfig {
    pub id: u16,
    pub endpoint: String,
    pub presign: String,
    pub bucket: String,
    pub access_key: String,
    pub secret: Option<Secret<String>>,
    pub cap_gb: u64,
}

#[derive(Clone, Debug, Default)]
pub struct MediaConfig {
    pub uploads: bool,
    pub stores: Vec<StoreConfig>,
    pub write: Option<u16>,
}

impl MediaConfig {
    pub fn from_env(keys_dir: &Path) -> Self {
        Self::parse(env_opt, keys_dir)
    }

    pub fn parse(get: impl Fn(&str) -> Option<String>, keys_dir: &Path) -> Self {
        let list = get("CHAT_STORES").unwrap_or_default();
        let stores = list
            .split(',')
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .filter_map(|raw| match raw.parse::<u16>() {
                Ok(id) => store(id, &get, keys_dir),
                Err(_) => {
                    tracing::warn!(store = raw, "CHAT_STORES holds a bad id, it is ignored");
                    None
                }
            })
            .collect();
        Self {
            uploads: get("CHAT_MEDIA_UPLOADS").is_some_and(|value| value.trim() == "true"),
            stores,
            write: get("CHAT_STORE_WRITE").and_then(|id| id.trim().parse().ok()),
        }
    }
}

fn store(id: u16, get: &impl Fn(&str) -> Option<String>, keys_dir: &Path) -> Option<StoreConfig> {
    let field = |name: &str| {
        get(&format!("CHAT_STORE_{id}_{name}")).filter(|value| !value.trim().is_empty())
    };
    let (Some(endpoint), Some(presign), Some(bucket), Some(access_key)) = (
        field("ENDPOINT"),
        field("PRESIGN"),
        field("BUCKET"),
        field("ACCESS_KEY"),
    ) else {
        tracing::warn!(store = id, "chat store is incomplete, it is ignored");
        return None;
    };
    Some(StoreConfig {
        id,
        endpoint,
        presign,
        bucket,
        access_key,
        secret: secret(id, keys_dir),
        cap_gb: field("CAP_GB")
            .and_then(|gb| gb.trim().parse().ok())
            .unwrap_or(0),
    })
}

fn secret(id: u16, keys_dir: &Path) -> Option<Secret<String>> {
    let raw = fs::read_to_string(keys_dir.join(format!("{SECRET_PREFIX}{id}"))).ok()?;
    let secret = raw.trim();
    (!secret.is_empty()).then(|| secret.to_owned().into())
}

#[cfg(test)]
#[path = "tests/media_test_support.rs"]
pub mod support;

#[cfg(test)]
#[path = "tests/media_rows.rs"]
pub mod rows;

#[cfg(test)]
#[path = "tests/media_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/media_routes_tests.rs"]
mod route_tests;

#[cfg(test)]
#[path = "tests/media_s3_live_tests.rs"]
mod s3_live;
