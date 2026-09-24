use std::path::{Path, PathBuf};

use super::*;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn create() -> io::Result<Self> {
        let path = std::env::temp_dir().join(format!("jobs-subscriptions-{}", Uuid::now_v7()));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _result = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn replace_publishes_only_the_complete_snapshot() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    let store = SnapshotStore::new(directory.path().to_owned(), 1_024);
    store.replace(b"old").await?;
    store.replace(b"new snapshot").await?;

    let loaded = store.load().await?;
    let StoredSnapshot::Found(bytes) = loaded else {
        return Err("snapshot is missing".into());
    };
    assert_eq!(bytes, b"new snapshot");
    Ok(())
}

#[tokio::test]
async fn oversized_replacement_keeps_last_good_snapshot() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = TestDirectory::create()?;
    let store = SnapshotStore::new(directory.path().to_owned(), 3);
    store.replace(b"old").await?;
    let replacement = store.replace(b"larger").await;

    assert!(matches!(replacement, Err(StoreError::TooLarge { .. })));
    let loaded = store.load().await?;
    let StoredSnapshot::Found(bytes) = loaded else {
        return Err("snapshot is missing".into());
    };
    assert_eq!(bytes, b"old");
    Ok(())
}

#[tokio::test]
async fn load_rejects_a_file_that_grew_past_the_limit() -> Result<(), Box<dyn std::error::Error>> {
    let directory = TestDirectory::create()?;
    let store = SnapshotStore::new(directory.path().to_owned(), 3);
    tokio::fs::write(directory.path().join(SNAPSHOT_FILE), b"four").await?;

    assert!(matches!(
        store.load().await,
        Err(StoreError::TooLarge { .. })
    ));
    Ok(())
}
