use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream::object_store::{GetErrorKind, ObjectStore};
use bytes::Bytes;
use futures::future::BoxFuture;
use tokio::io::AsyncReadExt;
use tokio::sync::OnceCell;

use crate::bus::nats::NatsService;

const TRANSFER_BUDGET: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectError {
    Unavailable,
    TimedOut,
}

pub trait Objects: Send + Sync {
    fn read<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Option<Bytes>, ObjectError>>;

    fn write<'a>(&'a self, name: &'a str, bytes: Bytes) -> BoxFuture<'a, Result<(), ObjectError>>;
}

pub struct NatsObjects {
    nats: Arc<NatsService>,
    bucket: &'static str,
    store: OnceCell<ObjectStore>,
}

impl ObjectError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::TimedOut => "timeout",
        }
    }
}

impl NatsObjects {
    pub fn new(nats: Arc<NatsService>, bucket: &'static str) -> Arc<Self> {
        Arc::new(Self {
            nats,
            bucket,
            store: OnceCell::new(),
        })
    }

    async fn store(&self) -> Result<&ObjectStore, ObjectError> {
        self.store
            .get_or_try_init(|| async {
                self.nats
                    .jetstream()
                    .get_object_store(self.bucket)
                    .await
                    .map_err(|_| ObjectError::Unavailable)
            })
            .await
    }

    async fn fetch(&self, name: &str) -> Result<Option<Bytes>, ObjectError> {
        let mut object = match self.store().await?.get(name).await {
            Ok(object) => object,
            Err(error) if error.kind() == GetErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(ObjectError::Unavailable),
        };
        let mut bytes = Vec::with_capacity(object.info().size);
        object
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| ObjectError::Unavailable)?;
        Ok(Some(Bytes::from(bytes)))
    }

    async fn store_bytes(&self, name: &str, bytes: Bytes) -> Result<(), ObjectError> {
        self.store()
            .await?
            .put(name, &mut bytes.as_ref())
            .await
            .map(|_| ())
            .map_err(|_| ObjectError::Unavailable)
    }
}

impl Objects for NatsObjects {
    fn read<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Option<Bytes>, ObjectError>> {
        Box::pin(async move {
            tokio::time::timeout(TRANSFER_BUDGET, self.fetch(name))
                .await
                .map_err(|_| ObjectError::TimedOut)?
        })
    }

    fn write<'a>(&'a self, name: &'a str, bytes: Bytes) -> BoxFuture<'a, Result<(), ObjectError>> {
        Box::pin(async move {
            tokio::time::timeout(TRANSFER_BUDGET, self.store_bytes(name, bytes))
                .await
                .map_err(|_| ObjectError::TimedOut)?
        })
    }
}
