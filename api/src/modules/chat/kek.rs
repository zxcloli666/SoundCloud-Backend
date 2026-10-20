use std::path::Path;

use uuid::Uuid;

use crate::modules::chat::{ChatError, ChatResult};

pub struct KekRing {
    current: u16,
    reports_current: u16,
}

impl KekRing {
    pub fn load(_dir: &Path, current: u16, reports_current: u16) -> anyhow::Result<Self> {
        Ok(Self {
            current,
            reports_current,
        })
    }

    pub fn current(&self) -> u16 {
        self.current
    }

    pub fn reports_current(&self) -> u16 {
        self.reports_current
    }

    pub fn wrap(&self, _conversation: Uuid, _epoch: i32, _ck: &[u8; 32]) -> ChatResult<Vec<u8>> {
        Err(ChatError::Unimplemented)
    }

    pub fn unwrap(
        &self,
        _kek_version: u16,
        _conversation: Uuid,
        _epoch: i32,
        _wrapped: &[u8],
    ) -> ChatResult<[u8; 32]> {
        Err(ChatError::Unimplemented)
    }

    pub fn seal_evidence(&self, _evidence: &[u8]) -> ChatResult<Vec<u8>> {
        Err(ChatError::Unimplemented)
    }

    pub fn open_evidence(&self, _kek_version: u16, _sealed: &[u8]) -> ChatResult<Vec<u8>> {
        Err(ChatError::Unimplemented)
    }
}
