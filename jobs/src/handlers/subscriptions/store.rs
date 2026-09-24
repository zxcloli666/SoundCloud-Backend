use std::io;
use std::path::{Path, PathBuf};

use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::warn;
use uuid::Uuid;

const SNAPSHOT_FILE: &str = "subscriptions.json";

pub(super) enum StoredSnapshot {
    Missing,
    Found(Vec<u8>),
}

pub(super) struct SnapshotStore {
    directory: PathBuf,
    snapshot_path: PathBuf,
    max_bytes: usize,
}

impl SnapshotStore {
    pub fn new(directory: PathBuf, max_bytes: usize) -> Self {
        let snapshot_path = directory.join(SNAPSHOT_FILE);
        Self {
            directory,
            snapshot_path,
            max_bytes,
        }
    }

    pub async fn load(&self) -> Result<StoredSnapshot, StoreError> {
        let file = match File::open(&self.snapshot_path).await {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(StoredSnapshot::Missing);
            }
            Err(error) => return Err(self.io_error("open", &self.snapshot_path, error)),
        };

        let metadata = file
            .metadata()
            .await
            .map_err(|error| self.io_error("inspect", &self.snapshot_path, error))?;
        if metadata.len() > self.max_bytes as u64 {
            return Err(StoreError::TooLarge {
                max_bytes: self.max_bytes,
            });
        }

        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        let mut limited = file.take(self.max_bytes as u64 + 1);
        limited
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| self.io_error("read", &self.snapshot_path, error))?;
        if bytes.len() > self.max_bytes {
            return Err(StoreError::TooLarge {
                max_bytes: self.max_bytes,
            });
        }

        Ok(StoredSnapshot::Found(bytes))
    }

    pub async fn verify_writable(&self) -> Result<(), StoreError> {
        fs::create_dir_all(&self.directory)
            .await
            .map_err(|error| self.io_error("create directory", &self.directory, error))?;
        let probe_path = self
            .directory
            .join(format!(".write-probe.{}", Uuid::now_v7()));
        let probe = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&probe_path)
            .await
            .map_err(|error| self.io_error("create write probe", &probe_path, error))?;
        probe
            .sync_all()
            .await
            .map_err(|error| self.io_error("sync write probe", &probe_path, error))?;
        drop(probe);
        fs::remove_file(&probe_path)
            .await
            .map_err(|error| self.io_error("remove write probe", &probe_path, error))?;
        Ok(())
    }

    pub fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    pub async fn replace(&self, bytes: &[u8]) -> Result<(), StoreError> {
        if bytes.len() > self.max_bytes {
            return Err(StoreError::TooLarge {
                max_bytes: self.max_bytes,
            });
        }

        fs::create_dir_all(&self.directory)
            .await
            .map_err(|error| self.io_error("create directory", &self.directory, error))?;
        let temporary_path = self.temporary_path();
        let result = self.write_and_replace(&temporary_path, bytes).await;
        if result.is_err() {
            self.discard_temporary(&temporary_path).await;
        }
        result
    }

    fn temporary_path(&self) -> PathBuf {
        self.directory
            .join(format!(".{SNAPSHOT_FILE}.{}.tmp", Uuid::now_v7()))
    }

    async fn write_and_replace(
        &self,
        temporary_path: &Path,
        bytes: &[u8],
    ) -> Result<(), StoreError> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(temporary_path)
            .await
            .map_err(|error| self.io_error("create temporary file", temporary_path, error))?;
        file.write_all(bytes)
            .await
            .map_err(|error| self.io_error("write temporary file", temporary_path, error))?;
        file.sync_all()
            .await
            .map_err(|error| self.io_error("sync temporary file", temporary_path, error))?;
        drop(file);
        fs::rename(temporary_path, &self.snapshot_path)
            .await
            .map_err(|error| self.io_error("replace", &self.snapshot_path, error))?;
        let directory = File::open(&self.directory)
            .await
            .map_err(|error| self.io_error("open directory", &self.directory, error))?;
        directory
            .sync_all()
            .await
            .map_err(|error| self.io_error("sync directory", &self.directory, error))?;
        Ok(())
    }

    async fn discard_temporary(&self, temporary_path: &Path) {
        match fs::remove_file(temporary_path).await {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                warn!(path = ?temporary_path, error = %error, "snapshot temporary file cleanup failed")
            }
        }
    }

    fn io_error(&self, operation: &'static str, path: &Path, source: io::Error) -> StoreError {
        StoreError::Io {
            operation,
            path: path.to_owned(),
            source,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(super) enum StoreError {
    #[error("snapshot exceeds its {max_bytes} byte limit")]
    TooLarge { max_bytes: usize },

    #[error("snapshot {operation} failed for {path:?}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl StoreError {
    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::TooLarge { .. })
    }
}

#[cfg(test)]
mod tests;
