#[cfg(test)]
#[path = "backfill_tests.rs"]
mod tests;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::{AUDIO_INDEX_QUARANTINE_SECONDS, REAP_RETRY_COOLDOWN_SECONDS};
use crate::handlers::star_priority::{
    LIKED_PRIORITY, Pass, Round, STAR_SCAN_WINDOW, Turn, split_next, with_next,
};

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
    let room = usize::try_from(room).unwrap_or(0);
    let mut round = Round::new("audio_index");
    let mut picked: Vec<AudioBackfill> = Vec::new();
    for tier in turn.tiers() {
        let left = room.saturating_sub(picked.len());
        let taken: Vec<String> = picked
            .iter()
            .map(|candidate| candidate.sc_track_id.clone())
            .collect();
        let found = match tier.pass() {
            Pass::Star(stored) => star(pool, left, stored, &taken).await?,
            Pass::Liked => by_priority(pool, left, LIKED_PRIORITY, LIKED_PRIORITY, &taken).await?,
            Pass::Rest => by_priority(pool, left, 0, i16::MAX, &taken).await?,
        };
        let (found, next) = split_next(found, left);
        round.record(
            tier.label(),
            found.len(),
            next.map(|candidate| candidate.waiting_since),
        );
        picked.extend(found);
    }
    round.report();
    Ok(picked)
}

async fn star(
    pool: &PgPool,
    left: usize,
    stored: i16,
    taken: &[String],
) -> Result<Vec<AudioBackfill>, sqlx::Error> {
    sqlx::query_file_as!(
        AudioBackfill,
        "queries/indexing/backfill_audio_star.sql",
        with_next(left),
        AUDIO_INDEX_QUARANTINE_SECONDS,
        REAP_RETRY_COOLDOWN_SECONDS,
        stored,
        taken,
        STAR_SCAN_WINDOW
    )
    .fetch_all(pool)
    .await
}

async fn by_priority(
    pool: &PgPool,
    left: usize,
    lowest: i16,
    highest: i16,
    taken: &[String],
) -> Result<Vec<AudioBackfill>, sqlx::Error> {
    sqlx::query_file_as!(
        AudioBackfill,
        "queries/indexing/backfill_audio.sql",
        with_next(left),
        AUDIO_INDEX_QUARANTINE_SECONDS,
        REAP_RETRY_COOLDOWN_SECONDS,
        lowest,
        highest,
        taken
    )
    .fetch_all(pool)
    .await
}
