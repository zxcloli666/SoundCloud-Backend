use serde_json::json;

use super::*;

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("spool-test-{}", uuid::Uuid::new_v4())))
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn events(spool: &Spool) -> Vec<SpooledEvent> {
    let mut events = Vec::new();
    for path in spool.queued().await.unwrap() {
        events.push(read_event(&path).await.unwrap().unwrap());
    }
    events
}

#[tokio::test]
async fn events_stay_queued_in_push_order_across_a_reopen() {
    let dir = ScratchDir::new();
    let spool = Spool::open(dir.0.clone()).await.unwrap();
    for track in ["1", "2", "3"] {
        spool
            .push("storage.track_uploaded", json!({ "sc_track_id": track }))
            .await
            .unwrap();
    }
    drop(spool);

    let reopened = Spool::open(dir.0.clone()).await.unwrap();
    reopened
        .push("storage.track_rejected", json!({ "sc_track_id": "4" }))
        .await
        .unwrap();

    let queued = reopened.queued().await.unwrap();
    let sequences: Vec<u64> = queued.iter().filter_map(|p| sequence_of(p)).collect();
    let events = events(&reopened).await;
    let tracks: Vec<&str> = events
        .iter()
        .filter_map(|e| e.payload["sc_track_id"].as_str())
        .collect();
    assert_eq!(sequences, [1, 2, 3, 4]);
    assert_eq!(tracks, ["1", "2", "3", "4"]);
    assert_eq!(events[3].subject, "storage.track_rejected");
}

#[tokio::test]
async fn every_event_gets_its_own_message_id() {
    let dir = ScratchDir::new();
    let spool = Spool::open(dir.0.clone()).await.unwrap();
    for _ in 0..50 {
        spool
            .push("storage.track_uploaded", json!({ "sc_track_id": "7" }))
            .await
            .unwrap();
    }

    let mut ids: Vec<String> = events(&spool).await.into_iter().map(|e| e.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 50);
}

#[tokio::test]
async fn a_write_cut_short_by_a_crash_is_dropped_on_open() {
    let dir = ScratchDir::new();
    std::fs::create_dir_all(&dir.0).unwrap();
    std::fs::write(dir.0.join("00000000000000000009-cut.part"), b"{\"id\"").unwrap();

    let spool = Spool::open(dir.0.clone()).await.unwrap();
    spool
        .push("storage.track_uploaded", json!({ "sc_track_id": "1" }))
        .await
        .unwrap();

    let names: Vec<String> = std::fs::read_dir(&dir.0)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .collect();
    assert_eq!(names.len(), 1);
    assert!(names[0].starts_with("00000000000000000001-"));
}

#[tokio::test]
async fn an_unreadable_event_is_set_aside_and_the_rest_stays_queued() {
    let dir = ScratchDir::new();
    std::fs::create_dir_all(&dir.0).unwrap();
    let broken = dir.0.join("00000000000000000001-broken.json");
    std::fs::write(&broken, b"not json").unwrap();
    let spool = Spool::open(dir.0.clone()).await.unwrap();
    spool
        .push("storage.track_uploaded", json!({ "sc_track_id": "2" }))
        .await
        .unwrap();

    assert_eq!(read_event(&broken).await.unwrap(), None);

    let queued = spool.queued().await.unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(sequence_of(&queued[0]), Some(2));
    assert!(broken.with_extension(UNREADABLE).exists());
}
