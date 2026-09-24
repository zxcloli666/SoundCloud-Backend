use std::sync::Arc;
use std::time::Duration;

use backend_contracts::{IndexTrackPayload, JobKind};
use mini_moka::sync::Cache;

use super::{BackgroundJob, BackgroundJobs};
use crate::error::AppResult;

const RECENT_CAPACITY: u64 = 65_536;
const RECENT_TTL: Duration = Duration::from_secs(15 * 60);

pub struct IndexingJobs {
    jobs: Arc<BackgroundJobs>,
    recent: Cache<String, ()>,
}

impl IndexingJobs {
    pub fn new(jobs: Arc<BackgroundJobs>) -> Arc<Self> {
        Arc::new(Self {
            jobs,
            recent: Cache::builder()
                .max_capacity(RECENT_CAPACITY)
                .time_to_live(RECENT_TTL)
                .build(),
        })
    }

    pub async fn enqueue(&self, sc_track_id: &str) -> AppResult<()> {
        let key = sc_track_id.to_owned();
        if self.recent.get(&key).is_some() {
            return Ok(());
        }

        let job = BackgroundJob::coalescing(
            JobKind::IndexTrack,
            sc_track_id,
            IndexTrackPayload {
                sc_track_id: sc_track_id.to_owned(),
            },
        )?;
        self.jobs.enqueue(&job).await?;
        self.recent.insert(key, ());
        Ok(())
    }
}
