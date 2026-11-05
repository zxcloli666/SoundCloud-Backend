use std::sync::{Mutex, PoisonError};

use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};

pub(super) const REST_WALK_WINDOW: i64 = 10_000;

#[derive(Default)]
pub(super) struct AlignWalk {
    after: Option<(DateTime<Utc>, String)>,
}

impl AlignWalk {
    pub(super) fn after_at(&self) -> Option<DateTime<Utc>> {
        self.after.as_ref().map(|(at, _)| *at)
    }

    pub(super) fn after_id(&self) -> Option<&str> {
        self.after.as_ref().map(|(_, id)| id.as_str())
    }

    pub(super) async fn moved_on(
        &mut self,
        transaction: &mut Transaction<'_, Postgres>,
        filled: bool,
    ) -> Result<(), sqlx::Error> {
        if filled {
            self.after = None;
            return Ok(());
        }
        let end = sqlx::query_file!(
            "queries/lyrics/reap_transcriptions_align_walk_end.sql",
            self.after_at(),
            self.after_id(),
            REST_WALK_WINDOW - 1
        )
        .fetch_optional(&mut **transaction)
        .await?;
        self.after = end.map(|end| (end.updated_at, end.sc_track_id));
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct SharedWalk(Mutex<AlignWalk>);

impl SharedWalk {
    pub(super) fn take(&self) -> AlignWalk {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }

    pub(super) fn put(&self, walk: AlignWalk) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = walk;
    }
}
