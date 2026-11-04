use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, anyhow, ensure};
use sc_chat_wire::seal::ResponseKey;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::modules::chat::{ChatError, ChatResult};

const EVIDENCE_LABEL: &[u8] = b"scd-chat/evidence/v1";

pub type KeyFiles = BTreeMap<u16, [u8; 32]>;

pub struct KekRing {
    current: u16,
    reports_current: u16,
    keks: KeyFiles,
    reports: KeyFiles,
}

enum KeyFile {
    Kek(u16),
    Reports(u16),
}

impl KekRing {
    pub fn load(dir: &Path, current: u16, reports_current: u16) -> anyhow::Result<Self> {
        let mut keks = KeyFiles::new();
        let mut reports = KeyFiles::new();
        let entries = fs::read_dir(dir)
            .with_context(|| format!("chat keys dir {} is not readable", dir.display()))?;
        for entry in entries {
            let entry = entry.context("chat keys dir is not readable")?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let (ring, version) = match key_file(&name) {
                Some(KeyFile::Kek(version)) => (&mut keks, version),
                Some(KeyFile::Reports(version)) => (&mut reports, version),
                None => continue,
            };
            ring.insert(version, read_key(&entry.path(), &name)?);
        }
        Self::new(current, reports_current, keks, reports)
    }

    pub fn new(
        current: u16,
        reports_current: u16,
        keks: KeyFiles,
        reports: KeyFiles,
    ) -> anyhow::Result<Self> {
        ensure!(
            keks.contains_key(&current),
            "the chat keys dir holds no kek-{current}"
        );
        ensure!(
            reports.contains_key(&reports_current),
            "the chat keys dir holds no reports-kek-{reports_current}"
        );
        Ok(Self {
            current,
            reports_current,
            keks,
            reports,
        })
    }

    pub fn current(&self) -> u16 {
        self.current
    }

    pub fn reports_current(&self) -> u16 {
        self.reports_current
    }

    pub fn wrap(&self, conversation: Uuid, epoch: i32, ck: &[u8; 32]) -> ChatResult<Vec<u8>> {
        let kek = self.keks.get(&self.current).ok_or(ChatError::Internal)?;
        sc_chat_wire::ck::wrap(
            kek,
            self.current,
            conversation.as_bytes(),
            wire_epoch(epoch)?,
            ck,
            fresh()?,
        )
        .map_err(|_| ChatError::Internal)
    }

    pub fn unwrap(
        &self,
        kek_version: u16,
        conversation: Uuid,
        epoch: i32,
        wrapped: &[u8],
    ) -> ChatResult<[u8; 32]> {
        let Some(kek) = self.keks.get(&kek_version) else {
            tracing::error!(
                kek_version,
                "a conversation key names a kek this host lacks"
            );
            return Err(ChatError::Internal);
        };
        sc_chat_wire::ck::unwrap(
            kek,
            kek_version,
            conversation.as_bytes(),
            wire_epoch(epoch)?,
            wrapped,
        )
        .map_err(|_| {
            tracing::error!(%conversation, epoch, kek_version, "a conversation key does not unwrap");
            ChatError::Internal
        })
    }

    pub fn seal_evidence(&self, evidence: &[u8]) -> ChatResult<Vec<u8>> {
        let key = self
            .reports
            .get(&self.reports_current)
            .ok_or(ChatError::Internal)?;
        ResponseKey::from_bytes(*key)
            .seal(&evidence_aad(self.reports_current), fresh()?, evidence)
            .map_err(|_| ChatError::Internal)
    }

    pub fn open_evidence(&self, kek_version: u16, sealed: &[u8]) -> ChatResult<Vec<u8>> {
        let Some(key) = self.reports.get(&kek_version) else {
            tracing::error!(
                kek_version,
                "report evidence names a reports kek this host lacks"
            );
            return Err(ChatError::Internal);
        };
        ResponseKey::from_bytes(*key)
            .open(&evidence_aad(kek_version), sealed)
            .map_err(|_| {
                tracing::error!(kek_version, "report evidence does not open");
                ChatError::Internal
            })
    }
}

pub fn wire_epoch(epoch: i32) -> ChatResult<u32> {
    u32::try_from(epoch).map_err(|_| ChatError::Internal)
}

fn fresh<const N: usize>() -> ChatResult<[u8; N]> {
    sc_chat_wire::random_bytes().map_err(|_| ChatError::Internal)
}

fn evidence_aad(version: u16) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(EVIDENCE_LABEL);
    digest.update(version.to_be_bytes());
    digest.finalize().into()
}

fn key_file(name: &str) -> Option<KeyFile> {
    if let Some(version) = name.strip_prefix("reports-kek-") {
        return version.parse().ok().map(KeyFile::Reports);
    }
    name.strip_prefix("kek-")?.parse().ok().map(KeyFile::Kek)
}

fn read_key(path: &Path, name: &str) -> anyhow::Result<[u8; 32]> {
    let bytes = fs::read(path).with_context(|| format!("{name} is not readable"))?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| anyhow!("{name} is not a 32 byte key"))
}

#[cfg(test)]
#[path = "tests/kek_tests.rs"]
mod tests;
