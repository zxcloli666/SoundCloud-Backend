#[cfg(test)]
#[path = "backfill_tests.rs"]
mod tests;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::{AUDIO_INDEX_QUARANTINE_SECONDS, REAP_RETRY_COOLDOWN_SECONDS};
use crate::handlers::star_priority::{Pass, Picks, Turn};

pub(super) struct AudioBackfill {
    pub(super) sc_track_id: String,
    pub(super) uploaded_generation: Option<i64>,
    waiting_since: DateTime<Utc>,
}

pub(super) async fn pick(
    pool: &PgPool,
    room: i64,
    turn: Turn,
) -> Result<Vec<AudioBackfill>, sqlx::Error> {
    let mut picks = Picks::new("audio_index");
    let mut picked: Vec<AudioBackfill> = Vec::new();
    for &tier in turn.tiers() {
        let left = room.saturating_sub(i64::try_from(picked.len()).unwrap_or(i64::MAX));
        if left <= 0 {
            break;
        }
        let taken: Vec<String> = picked
            .iter()
            .map(|candidate| candidate.sc_track_id.clone())
            .collect();
        let found = match tier.pass() {
            Pass::Star(stored) => {
                sqlx::query_file_as!(
                    AudioBackfill,
                    "queries/indexing/backfill_audio_star.sql",
                    left,
                    AUDIO_INDEX_QUARANTINE_SECONDS,
                    REAP_RETRY_COOLDOWN_SECONDS,
                    stored,
                    &taken
                )
                .fetch_all(pool)
                .await?
            }
            Pass::UpToPriority(highest) => {
                sqlx::query_file_as!(
                    AudioBackfill,
                    "queries/indexing/backfill_audio.sql",
                    left,
                    AUDIO_INDEX_QUARANTINE_SECONDS,
                    REAP_RETRY_COOLDOWN_SECONDS,
                    highest,
                    &taken
                )
                .fetch_all(pool)
                .await?
            }
        };
        picks.record(
            tier.label(),
            found.iter().map(|candidate| candidate.waiting_since),
        );
        picked.extend(found);
    }
    picks.report();
    Ok(picked)
}
