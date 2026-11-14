use std::time::SystemTime;

use backend_contracts::pipeline::SayRequest;
use bytes::Bytes;
use futures::future::BoxFuture;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    Unavailable,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub request: SayRequest,
    pub reply_to: String,
    pub ticket: Uuid,
    pub deadline: SystemTime,
}

pub trait Lane: Send + Sync {
    fn depth(&self) -> BoxFuture<'_, Result<u64, StoreError>>;

    fn ask(&self, ask: Ask) -> BoxFuture<'_, Result<(), StoreError>>;
}

pub trait Replies: Send + Sync {
    fn last<'a>(&'a self, subject: &'a str) -> BoxFuture<'a, Result<Option<Bytes>, StoreError>>;

    fn put<'a>(&'a self, subject: &'a str, payload: Bytes)
    -> BoxFuture<'a, Result<(), StoreError>>;
}

pub trait Objects: Send + Sync {
    fn read<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Option<Bytes>, StoreError>>;

    fn write<'a>(&'a self, name: &'a str, bytes: Bytes) -> BoxFuture<'a, Result<(), StoreError>>;
}

impl StoreError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::TimedOut => "timeout",
        }
    }
}
