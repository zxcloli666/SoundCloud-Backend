use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, anyhow, ensure};
use chat_wire::ock::{OckSecret, pinned};

pub struct OckRing {
    current: u8,
    keys: BTreeMap<u8, OckSecret>,
}

impl OckRing {
    pub fn load(dir: &Path, current: u8) -> anyhow::Result<Self> {
        let mut keys = BTreeMap::new();
        let entries = fs::read_dir(dir)
            .with_context(|| format!("chat keys dir {} is not readable", dir.display()))?;
        for entry in entries {
            let entry = entry.context("chat keys dir is not readable")?;
            let Some(version) = ock_version(&entry.file_name().to_string_lossy()) else {
                continue;
            };
            let bytes =
                fs::read(entry.path()).with_context(|| format!("ock-{version} is not readable"))?;
            let secret = OckSecret::from_bytes(&bytes)
                .map_err(|_| anyhow!("ock-{version} is not a 32 byte X25519 key"))?;
            keys.insert(version, secret);
        }
        Self::new(current, keys)
    }

    pub fn new(current: u8, keys: BTreeMap<u8, OckSecret>) -> anyhow::Result<Self> {
        ensure!(
            keys.contains_key(&current),
            "the chat keys dir holds no ock-{current}"
        );
        for (version, secret) in &keys {
            if pinned(*version) != Some(&secret.public()) {
                tracing::warn!(version, "ock key does not match the chat-wire pin");
            }
        }
        Ok(Self { current, keys })
    }

    pub fn current(&self) -> u8 {
        self.current
    }

    pub fn get(&self, version: u8) -> Option<&OckSecret> {
        self.keys.get(&version)
    }
}

fn ock_version(file_name: &str) -> Option<u8> {
    file_name.strip_prefix("ock-")?.parse().ok()
}
