use chrono::{DateTime, Utc};
use sqlx::{Postgres, Transaction};

use super::TranscriptionCandidate;
use crate::handlers::star_priority::{Pass, Picks, Tier, Turns};

struct EmbeddingCandidate {
    sc_track_id: String,
    waiting_since: DateTime<Utc>,
}

struct AlignCandidate {
    sc_track_id: String,
    uploaded_generation: i64,
    waiting_since: DateTime<Utc>,
}

pub(super) async fn embeddings(
    transaction: &mut Transaction<'_, Postgres>,
    room: i64,
    turns: &Turns,
) -> Result<Vec<(String, Tier)>, sqlx::Error> {
    if room <= 0 {
        return Ok(Vec::new());
    }
    let turn = turns.next();
    let mut picks = Picks::new("lyrics_embed");
    let mut picked: Vec<(String, Tier)> = Vec::new();
    for &tier in turn.tiers() {
        let left = room.saturating_sub(i64::try_from(picked.len()).unwrap_or(i64::MAX));
        if left <= 0 {
            break;
        }
        let taken: Vec<String> = picked.iter().map(|(id, _)| id.clone()).collect();
        let found = match tier.pass() {
            Pass::Star(stored) => {
                sqlx::query_file_as!(
                    EmbeddingCandidate,
                    "queries/lyrics/reap_embeddings_star.sql",
                    left,
                    stored,
                    &taken
                )
                .fetch_all(&mut **transaction)
                .await?
            }
            Pass::UpToPriority(highest) => {
                sqlx::query_file_as!(
                    EmbeddingCandidate,
                    "queries/lyrics/reap_embeddings.sql",
                    left,
                    highest,
                    &taken
                )
                .fetch_all(&mut **transaction)
                .await?
            }
        };
        picks.record(tier.label(), found.iter().map(|found| found.waiting_since));
        picked.extend(found.into_iter().map(|found| (found.sc_track_id, tier)));
    }
    picks.report();
    Ok(picked)
}

pub(super) async fn alignments(
    transaction: &mut Transaction<'_, Postgres>,
    room: i64,
    turns: &Turns,
) -> Result<Vec<(TranscriptionCandidate, Tier)>, sqlx::Error> {
    if room <= 0 {
        return Ok(Vec::new());
    }
    let turn = turns.next();
    let mut picks = Picks::new("lyrics_align");
    let mut picked: Vec<(TranscriptionCandidate, Tier)> = Vec::new();
    for &tier in turn.tiers() {
        let left = room.saturating_sub(i64::try_from(picked.len()).unwrap_or(i64::MAX));
        if left <= 0 {
            break;
        }
        let taken: Vec<String> = picked
            .iter()
            .map(|(candidate, _)| candidate.sc_track_id.clone())
            .collect();
        let found = match tier.pass() {
            Pass::Star(stored) => {
                sqlx::query_file_as!(
                    AlignCandidate,
                    "queries/lyrics/reap_transcriptions_align_star.sql",
                    left,
                    stored,
                    &taken
                )
                .fetch_all(&mut **transaction)
                .await?
            }
            Pass::UpToPriority(highest) => {
                sqlx::query_file_as!(
                    AlignCandidate,
                    "queries/lyrics/reap_transcriptions_align.sql",
                    left,
                    highest,
                    &taken
                )
                .fetch_all(&mut **transaction)
                .await?
            }
        };
        picks.record(tier.label(), found.iter().map(|found| found.waiting_since));
        picked.extend(found.into_iter().map(|found| {
            let candidate = TranscriptionCandidate {
                sc_track_id: found.sc_track_id,
                uploaded_generation: found.uploaded_generation,
            };
            (candidate, tier)
        }));
    }
    picks.report();
    Ok(picked)
}
