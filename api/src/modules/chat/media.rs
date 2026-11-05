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
    pub moving: Option<(u16, u16)>,
}

impl MediaConfig {
    pub fn from_env(keys_dir: &Path) -> anyhow::Result<Self> {
        Self::parse(env_opt, keys_dir)
    }

    pub fn parse(get: impl Fn(&str) -> Option<String>, keys_dir: &Path) -> anyhow::Result<Self> {
        let uploads = get("CHAT_MEDIA_UPLOADS").is_some_and(|value| value.trim() == "true");
        let mut faults = Vec::new();
        let mut stores = Vec::new();
        let list = get("CHAT_STORES").unwrap_or_default();
        for raw in list.split(',').map(str::trim).filter(|id| !id.is_empty()) {
            match raw.parse::<u16>() {
                Ok(id) => stores.extend(store(id, &get, keys_dir, &mut faults)),
                Err(_) => faults.push(format!("CHAT_STORES holds a bad id {raw:?}")),
            }
        }
        let write = write_store(&get, uploads, &mut faults);
        shared_buckets(&stores, &mut faults);
        let moving = move_pair(&get, &stores, &mut faults);
        if uploads && !faults.is_empty() {
            anyhow::bail!(
                "CHAT_MEDIA_UPLOADS is on but the chat stores are misconfigured: {}",
                faults.join("; ")
            );
        }
        for fault in &faults {
            tracing::warn!(fault = fault.as_str(), "a chat store setting is ignored");
        }
        if moving.is_some() && !faults.is_empty() {
            tracing::warn!("CHAT_MEDIA_MOVE stays off while a chat store setting is ignored");
        }
        Ok(Self {
            uploads,
            stores,
            write,
            moving: moving.filter(|_| faults.is_empty()),
        })
    }
}

fn store(
    id: u16,
    get: &impl Fn(&str) -> Option<String>,
    keys_dir: &Path,
    faults: &mut Vec<String>,
) -> Option<StoreConfig> {
    let field = |name: &str| {
        get(&format!("CHAT_STORE_{id}_{name}")).filter(|value| !value.trim().is_empty())
    };
    let (Some(endpoint), Some(presign), Some(bucket), Some(access_key)) = (
        field("ENDPOINT"),
        field("PRESIGN"),
        field("BUCKET"),
        field("ACCESS_KEY"),
    ) else {
        faults.push(format!(
            "store {id} needs CHAT_STORE_{id}_ENDPOINT, _PRESIGN, _BUCKET and _ACCESS_KEY"
        ));
        return None;
    };
    let secret = secret(id, keys_dir);
    if secret.is_none() {
        faults.push(format!(
            "store {id} has no secret in {}",
            keys_dir.join(format!("{SECRET_PREFIX}{id}")).display()
        ));
    }
    let cap_gb = field("CAP_GB").and_then(|gb| gb.trim().parse().ok());
    if cap_gb.is_none() {
        faults.push(format!(
            "CHAT_STORE_{id}_CAP_GB is missing or not a whole number of GB"
        ));
    }
    Some(StoreConfig {
        id,
        endpoint,
        presign,
        bucket,
        access_key,
        secret,
        cap_gb: cap_gb.unwrap_or(0),
    })
}

fn write_store(
    get: &impl Fn(&str) -> Option<String>,
    uploads: bool,
    faults: &mut Vec<String>,
) -> Option<u16> {
    let raw = get("CHAT_STORE_WRITE").unwrap_or_default();
    let raw = raw.trim();
    if raw.is_empty() {
        if uploads {
            faults.push("CHAT_STORE_WRITE is not set".to_owned());
        }
        return None;
    }
    let parsed = raw.parse().ok();
    if parsed.is_none() {
        faults.push(format!("CHAT_STORE_WRITE {raw:?} is not a store id"));
    }
    parsed
}

fn shared_buckets(stores: &[StoreConfig], faults: &mut Vec<String>) {
    for (index, store) in stores.iter().enumerate() {
        if let Some(first) = stores[..index]
            .iter()
            .find(|other| other.bucket == store.bucket)
        {
            faults.push(format!(
                "stores {} and {} share bucket {:?}, reconcile and the mover stay off",
                first.id, store.id, store.bucket
            ));
        }
    }
}

fn move_pair(
    get: &impl Fn(&str) -> Option<String>,
    stores: &[StoreConfig],
    faults: &mut Vec<String>,
) -> Option<(u16, u16)> {
    let raw = get("CHAT_MEDIA_MOVE").unwrap_or_default();
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let pair = raw.split_once(':').and_then(|(from, to)| {
        let pair: (u16, u16) = (from.trim().parse().ok()?, to.trim().parse().ok()?);
        (pair.0 != pair.1).then_some(pair)
    });
    let Some((from, to)) = pair else {
        faults.push(format!(
            "CHAT_MEDIA_MOVE {raw:?} is not <from>:<to> of two stores"
        ));
        return None;
    };
    let listed = |id: u16| stores.iter().any(|store| store.id == id);
    if !listed(from) || !listed(to) {
        faults.push(format!(
            "CHAT_MEDIA_MOVE {raw:?} names a store CHAT_STORES does not list"
        ));
        return None;
    }
    Some((from, to))
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
#[path = "tests/pg_cut_test_support.rs"]
pub mod pg_cut;

#[cfg(test)]
#[path = "tests/media_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/media_routes_tests.rs"]
mod route_tests;

#[cfg(test)]
#[path = "tests/media_s3_live_tests.rs"]
mod s3_live;
