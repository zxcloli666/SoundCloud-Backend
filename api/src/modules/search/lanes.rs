use std::time::Duration;

use futures::future::join_all;
use sqlx::{Postgres, Transaction};
use tokio::sync::SemaphorePermit;

use super::catalog::SearchService;
use super::resolve::Resolved;
use crate::error::AppResult;

pub const SPARE_LANES: usize = 6;
const SPARE_WAIT: Duration = Duration::from_millis(100);

pub struct Lane<'a> {
    pub tx: Transaction<'static, Postgres>,
    pub found: Vec<Resolved>,
    _spare: Option<SemaphorePermit<'a>>,
}

impl SearchService {
    pub(super) async fn lanes(&self, wanted: usize) -> AppResult<Vec<Lane<'_>>> {
        let spare = (1..wanted)
            .map_while(|_| self.spare_lanes.try_acquire().ok())
            .map(|held| async move {
                let tx = tokio::time::timeout(SPARE_WAIT, self.begin())
                    .await
                    .ok()?
                    .ok()?;
                Some(Lane {
                    tx,
                    found: Vec::new(),
                    _spare: Some(held),
                })
            });
        let (own, spare) = tokio::join!(self.begin(), join_all(spare));
        let mut lanes = vec![Lane {
            tx: own?,
            found: Vec::new(),
            _spare: None,
        }];
        lanes.extend(spare.into_iter().flatten());
        Ok(lanes)
    }
}
