use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_nats::HeaderMap;
use async_nats::header::NATS_MESSAGE_ID;
use async_nats::jetstream::Context;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;
use tracing::{debug, error, warn};

const ACK_TIMEOUT: Duration = Duration::from_secs(5);
const FIRST_RETRY: Duration = Duration::from_secs(1);
const LAST_RETRY: Duration = Duration::from_secs(30);
const IDLE_RECHECK: Duration = Duration::from_secs(30);
const QUEUED: &str = "json";
const PARTIAL: &str = "part";
const UNREADABLE: &str = "bad";

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct SpooledEvent {
    pub id: String,
    pub subject: String,
    pub payload: serde_json::Value,
}

pub struct Spool {
    dir: PathBuf,
    next: AtomicU64,
    wake: Notify,
}

impl Spool {
    pub async fn open(dir: PathBuf) -> io::Result<Self> {
        tokio::fs::create_dir_all(&dir).await?;
        remove_partial_writes(&dir).await?;
        let last = queued_in(&dir)
            .await?
            .last()
            .and_then(|path| sequence_of(path))
            .unwrap_or(0);
        Ok(Self {
            dir,
            next: AtomicU64::new(last + 1),
            wake: Notify::new(),
        })
    }

    pub async fn push(&self, subject: &str, payload: serde_json::Value) -> io::Result<()> {
        let event = SpooledEvent {
            id: uuid::Uuid::new_v4().to_string(),
            subject: subject.to_owned(),
            payload,
        };
        let sequence = self.next.fetch_add(1, Ordering::Relaxed);
        let path = self
            .dir
            .join(format!("{sequence:020}-{}.{QUEUED}", event.id));
        let partial = path.with_extension(PARTIAL);
        write_synced(&partial, &serde_json::to_vec(&event)?).await?;
        tokio::fs::rename(&partial, &path).await?;
        sync_dir(&self.dir).await?;
        self.wake.notify_one();
        Ok(())
    }

    pub async fn queued(&self) -> io::Result<Vec<PathBuf>> {
        queued_in(&self.dir).await
    }

    pub async fn run(self: Arc<Self>, jetstream: Context) {
        let mut retry = FIRST_RETRY;
        loop {
            match self.publish_queued(&jetstream).await {
                Ok(()) => {
                    retry = FIRST_RETRY;
                    let _ = tokio::time::timeout(IDLE_RECHECK, self.wake.notified()).await;
                }
                Err(failure) => {
                    let waiting = self.queued().await.map_or(0, |queued| queued.len());
                    warn!("[spool] {waiting} storage events wait for NATS: {failure}");
                    tokio::time::sleep(retry).await;
                    retry = (retry * 2).min(LAST_RETRY);
                }
            }
        }
    }

    async fn publish_queued(&self, jetstream: &Context) -> Result<(), String> {
        let queued = self
            .queued()
            .await
            .map_err(|e| format!("spool {} is unreadable: {e}", self.dir.display()))?;
        for path in queued {
            let Some(event) = read_event(&path).await? else {
                continue;
            };
            publish(jetstream, &event).await?;
            tokio::fs::remove_file(&path)
                .await
                .map_err(|e| format!("{} stays queued: {e}", path.display()))?;
        }
        Ok(())
    }
}

async fn publish(jetstream: &Context, event: &SpooledEvent) -> Result<(), String> {
    let mut headers = HeaderMap::new();
    headers.insert(NATS_MESSAGE_ID, event.id.as_str());
    let body = serde_json::to_vec(&event.payload).map_err(|e| e.to_string())?;
    let acknowledged = tokio::time::timeout(ACK_TIMEOUT, async {
        jetstream
            .publish_with_headers(event.subject.clone(), headers, body.into())
            .await?
            .await
    })
    .await
    .map_err(|_| {
        format!(
            "{} {} was not acknowledged in time",
            event.subject, event.id
        )
    })?
    .map_err(|e| format!("{} {}: {e}", event.subject, event.id))?;
    if acknowledged.duplicate {
        debug!(
            "[spool] {} {} was already stored in {}",
            event.subject, event.id, acknowledged.stream
        );
    }
    Ok(())
}

async fn read_event(path: &Path) -> Result<Option<SpooledEvent>, String> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("{} is unreadable: {e}", path.display()))?;
    match serde_json::from_slice(&bytes) {
        Ok(event) => Ok(Some(event)),
        Err(e) => {
            error!(
                "[spool] {} is not an event ({e}); set aside as .{UNREADABLE}",
                path.display()
            );
            tokio::fs::rename(path, path.with_extension(UNREADABLE))
                .await
                .map_err(|e| format!("{} could not be set aside: {e}", path.display()))?;
            Ok(None)
        }
    }
}

async fn queued_in(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut queued = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == QUEUED) {
            queued.push(path);
        }
    }
    queued.sort();
    Ok(queued)
}

async fn remove_partial_writes(dir: &Path) -> io::Result<()> {
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == PARTIAL) {
            tokio::fs::remove_file(&path).await?;
        }
    }
    Ok(())
}

fn sequence_of(path: &Path) -> Option<u64> {
    let name = path.file_name()?.to_str()?;
    name.split_once('-')?.0.parse().ok()
}

async fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = tokio::fs::File::create(path).await?;
    file.write_all(bytes).await?;
    file.sync_all().await
}

async fn sync_dir(dir: &Path) -> io::Result<()> {
    tokio::fs::File::open(dir).await?.sync_all().await
}

#[cfg(test)]
#[path = "spool_tests.rs"]
mod tests;
