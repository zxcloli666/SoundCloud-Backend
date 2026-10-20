use uuid::Uuid;

use crate::modules::chat::ChatResult;
use crate::modules::chat::kek::KekRing;

pub trait ChatKeys: Send + Sync {
    fn kek_current(&self) -> u16;
    fn reports_current(&self) -> u16;
    fn wrap_ck(&self, conversation: Uuid, epoch: i32, ck: &[u8; 32]) -> ChatResult<Vec<u8>>;
    fn unwrap_ck(
        &self,
        kek_version: u16,
        conversation: Uuid,
        epoch: i32,
        wrapped: &[u8],
    ) -> ChatResult<[u8; 32]>;
    fn seal_evidence(&self, evidence: &[u8]) -> ChatResult<Vec<u8>>;
    fn open_evidence(&self, version: u16, sealed: &[u8]) -> ChatResult<Vec<u8>>;
}

impl ChatKeys for KekRing {
    fn kek_current(&self) -> u16 {
        self.current()
    }

    fn reports_current(&self) -> u16 {
        KekRing::reports_current(self)
    }

    fn wrap_ck(&self, conversation: Uuid, epoch: i32, ck: &[u8; 32]) -> ChatResult<Vec<u8>> {
        self.wrap(conversation, epoch, ck)
    }

    fn unwrap_ck(
        &self,
        kek_version: u16,
        conversation: Uuid,
        epoch: i32,
        wrapped: &[u8],
    ) -> ChatResult<[u8; 32]> {
        self.unwrap(kek_version, conversation, epoch, wrapped)
    }

    fn seal_evidence(&self, evidence: &[u8]) -> ChatResult<Vec<u8>> {
        KekRing::seal_evidence(self, evidence)
    }

    fn open_evidence(&self, version: u16, sealed: &[u8]) -> ChatResult<Vec<u8>> {
        KekRing::open_evidence(self, version, sealed)
    }
}
